// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};
use trames_linux::{GuestOutput, OutputFd, TerminalEvent};
use trames_session::{MemoryLimits, RuntimeOptions, Terminal};
type Runtime = trames_session::Runtime<trames_hvf::kernel::Cpu>;

struct Output {
    host_memory_total: Option<usize>,
    deadline: Option<Instant>,
    window: Option<(u16, u16)>,
    pressure_target: Option<u64>,
    next_admin_check: Instant,
    bridge: Option<std::sync::Arc<super::windows::Bridge>>,
}
impl Output {
    fn poll_admin(&mut self) {
        if Instant::now() >= self.next_admin_check {
            super::lifecycle::poll_stop();
            self.next_admin_check = Instant::now() + Duration::from_millis(100);
        }
    }
    fn observe_pressure(&mut self) -> io::Result<()> {
        match trames_hvf::take_memory_pressure().map_err(io::Error::other)? {
            1 => self.pressure_target = None,
            level if level & 6 != 0 => {
                let mapped = trames_hvf::paging_stats().mapped_bytes;
                self.pressure_target = Some(if level & 4 != 0 {
                    mapped / 4
                } else {
                    mapped / 4 * 3
                });
            }
            _ => {}
        }
        Ok(())
    }
}
fn window_size() -> Option<(u16, u16)> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ writes exactly one initialized winsize structure.
    (unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } == 0)
        .then_some((size.ws_row, size.ws_col))
        .filter(|&(rows, columns)| rows != 0 && columns != 0)
}
impl GuestOutput for Output {
    fn host_memory(&self) -> Option<trames_linux::HostMemoryInfo> {
        self.host_memory_total.map(super::host_memory::snapshot)
    }
    fn poll_control(&mut self, _: &trames_vfs::Vfs) -> io::Result<bool> {
        self.observe_pressure()?;
        self.poll_admin();
        Ok(super::lifecycle::stopping()
            || self.bridge.as_ref().is_some_and(|b| b.stopped())
            || self.deadline.is_some_and(|d| Instant::now() >= d)
            || self.pressure_target.is_some())
    }
    fn control(&mut self, _: &mut trames_vfs::Vfs) -> io::Result<bool> {
        self.poll_admin();
        if super::lifecycle::stopping()
            || self.bridge.as_ref().is_some_and(|b| b.stopped())
            || self.deadline.is_some_and(|d| Instant::now() >= d)
        {
            return Ok(false);
        }
        self.observe_pressure()?;
        if let Some(target) = self.pressure_target {
            let remaining = trames_hvf::paging_stats()
                .mapped_bytes
                .saturating_sub(target);
            if remaining != 0 {
                trames_hvf::reclaim(remaining.min(8 << 20)).map_err(io::Error::other)?;
            }
            if trames_hvf::paging_stats().mapped_bytes <= target {
                self.pressure_target = None;
            }
        }
        Ok(true)
    }
    fn write(&mut self, fd: OutputFd, bytes: &[u8]) -> io::Result<usize> {
        match fd {
            OutputFd::Stdout => {
                let mut out = io::stdout().lock();
                let n = out.write(bytes)?;
                out.flush()?;
                Ok(n)
            }
            OutputFd::Stderr => {
                let mut out = io::stderr().lock();
                let n = out.write(bytes)?;
                out.flush()?;
                Ok(n)
            }
        }
    }
    fn stdin_ready(&mut self) -> io::Result<bool> {
        let mut fd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd; zero timeout never blocks the guest scheduler.
        let result = unsafe { libc::poll(&mut fd, 1, 0) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result > 0)
        }
    }
    fn read_stdin(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.poll_stdin(bytes)
    }
    fn poll_stdin(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if !self.stdin_ready()? {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        // SAFETY: writable output slice; readiness was checked above. Read the
        // fd directly so a stdio buffer cannot hide bytes from the next poll.
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
    fn terminal_event(&mut self) -> Option<TerminalEvent> {
        let size = window_size()?;
        if self.window == Some(size) {
            return None;
        }
        self.window = Some(size);
        Some(TerminalEvent::Resize {
            rows: size.0,
            columns: size.1,
        })
    }
}

/// Keep host terminal setup local; no subprocess or guest state is involved.
struct RawTerminal(Option<libc::termios>);
impl RawTerminal {
    fn enter(enabled: bool) -> io::Result<Self> {
        if !enabled || !io::stdin().is_terminal() {
            return Ok(Self(None));
        }
        let mut saved = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr initializes one complete termios on success.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, saved.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful call initialized all fields.
        let saved = unsafe { saved.assume_init() };
        let mut raw = saved;
        // SAFETY: private initialized termios; tcsetattr reads it synchronously.
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(Some(saved)))
    }
}
impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Some(saved) = &self.0 {
            // SAFETY: original initialized settings; the fd can legitimately
            // have hung up, in which case there is no live terminal to restore.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved);
            }
        }
    }
}

pub struct Command {
    rootfs: Option<String>,
    storage: Option<String>,
    imports: Vec<String>,
    mounts: Vec<(String, bool)>,
    filesystem: bool,
    network: trames_vfs::NetworkMode,
    trace: bool,
    metrics: bool,
    timeout: u64,
    limit: usize,
    cpus: usize,
    resident_limit: Option<u64>,
    pub windows: bool,
    vulkan: bool,
    video: bool,
    pty: bool,
    argv: Vec<String>,
}
impl Command {
    pub fn parse(
        arguments: impl Iterator<Item = String>,
        pty: bool,
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let mut arguments = arguments.peekable();
        let mut rootfs = None;
        let mut storage = None;
        let mut imports = Vec::new();
        let mut mounts = Vec::new();
        let mut filesystem = false;
        let mut network = trames_vfs::NetworkMode::Full;
        let mut trace = false;
        let mut metrics = false;
        let mut timeout = 0;
        let mut limit = MemoryLimits::host_managed().total;
        let mut cpus = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(8)
            .min(trames_hvf::max_parallel_cpus());
        let mut resident_limit = None;
        let mut windows = true;
        let mut vulkan = false;
        let mut video = false;
        let mut pty = pty;
        while arguments.peek().is_some_and(|a| a.starts_with("--")) {
            match arguments.next().unwrap().as_str() {
                "--rootfs" => rootfs = Some(arguments.next().ok_or("missing --rootfs archive")?),
                "--storage" => storage = Some(arguments.next().ok_or("missing storage directory")?),
                "--exec-file" => imports.push(arguments.next().ok_or("missing GUEST=HOST import")?),
                "--mount" => {
                    mounts.push((arguments.next().ok_or("missing GUEST=HOST mount")?, false))
                }
                "--mount-ro" => {
                    mounts.push((arguments.next().ok_or("missing GUEST=HOST mount")?, true))
                }
                "--fs" => {
                    filesystem = true;
                    windows = false;
                }
                "--network" => {
                    network = match arguments.next().as_deref() {
                        Some("off") => trames_vfs::NetworkMode::Off,
                        Some("loopback") => trames_vfs::NetworkMode::Loopback,
                        Some("outbound") => trames_vfs::NetworkMode::Outbound,
                        Some("full") => trames_vfs::NetworkMode::Full,
                        _ => return Err("invalid network mode".into()),
                    }
                }
                "--timeout" => timeout = arguments.next().ok_or("missing timeout")?.parse()?,
                "--cpus" => {
                    cpus = arguments.next().ok_or("missing CPU count")?.parse()?;
                    if !(1..=trames_hvf::max_parallel_cpus()).contains(&cpus) {
                        return Err(format!(
                            "CPU count must be 1..{}",
                            trames_hvf::max_parallel_cpus()
                        )
                        .into());
                    }
                }
                "--resident-mib" => {
                    resident_limit = Some(
                        arguments
                            .next()
                            .ok_or("missing resident target")?
                            .parse::<u64>()?
                            .checked_mul(1 << 20)
                            .ok_or("resident target overflow")?,
                    );
                }
                "--memory-mib" => {
                    let value = arguments.next().ok_or("missing memory limit")?;
                    limit = if value == "auto" {
                        MemoryLimits::host_managed().total
                    } else {
                        value
                            .parse::<usize>()?
                            .checked_mul(1 << 20)
                            .ok_or("memory limit overflow")?
                    }
                }
                "--no-windows" => windows = false,
                "--vulkan" => vulkan = true,
                "--video" => video = true,
                "--trace" => trace = true,
                "--metrics" => metrics = true,
                "--pty" => pty = true,
                "--no-pty" => pty = false,
                "--" => break,
                "--help" => {
                    println!(
                        "trames-mac-run [--rootfs Alpine.tar.gz] [--storage DIR] [--exec-file GUEST=HOST] [--mount GUEST=HOST] [--mount-ro GUEST=HOST] [--fs ls|cat|get|put ...] [--network off|loopback|outbound|full] [--memory-mib auto|N] [--cpus N] [--resident-mib N] [--timeout SECONDS] [--pty|--no-pty] [--no-windows] [--vulkan] [--video] [--trace] [--metrics] [ELF [ARG...]]\nLinux userspace with hardware ARM64 execution; defaults to host-managed memory and /bin/sh -l with a rootfs."
                    );
                    return Ok(None);
                }
                _ => return Err("unknown option".into()),
            }
        }
        let mut argv: Vec<String> = arguments.collect();
        if argv.is_empty() && !filesystem && (rootfs.is_some() || storage.is_some()) {
            argv = vec!["/bin/sh".into(), "-l".into()];
        }
        Ok(Some(Self {
            rootfs,
            storage,
            imports,
            mounts,
            filesystem,
            network,
            trace,
            metrics,
            timeout,
            limit,
            cpus,
            resident_limit,
            windows,
            vulkan,
            video,
            pty,
            argv,
        }))
    }
}
pub fn run(
    bridge: Option<std::sync::Arc<super::windows::Bridge>>,
    command: Command,
) -> Result<u8, Box<dyn std::error::Error>> {
    let Command {
        rootfs,
        storage,
        imports,
        mounts,
        filesystem,
        network,
        trace,
        metrics,
        timeout,
        limit,
        cpus,
        resident_limit,
        windows,
        vulkan,
        video,
        pty,
        argv,
    } = command;
    let path = argv
        .first()
        .ok_or("an ELF program or --rootfs is required; use --help")?;
    let options = RuntimeOptions {
        vulkan_accelerator: vulkan,
        video_accelerator: video,
        memory_limits: MemoryLimits {
            per_process: limit,
            total: limit,
        },
        ..Default::default()
    };
    options.memory_limits.validate()?;
    let mut vfs = if let Some(directory) = &storage {
        trames_vfs::Vfs::open_disk(directory, trames_vfs::DiskOptions::default())?
    } else {
        trames_vfs::Vfs::default()
    };
    if !filesystem {
        vfs.configure_network(network)?;
    }
    if let Some(archive) = &rootfs {
        if vfs
            .stat_at(trames_vfs::AT_FDCWD, b"/etc/alpine-release")
            .is_err()
        {
            trames_distro::install_alpine(&mut std::fs::File::open(archive)?, &mut vfs)?;
        }
    }
    // Use the runtime's existing host-resolver endpoint, as the iPad and CLI
    // products do. Never replace a resolver file configured by the guest user.
    if !filesystem
        && vfs
            .stat_at(trames_vfs::AT_FDCWD, b"/etc/resolv.conf")
            .is_err_and(|e| e.0 == 2)
    {
        if vfs
            .stat_at(trames_vfs::AT_FDCWD, b"/etc")
            .is_err_and(|e| e.0 == 2)
        {
            vfs.mkdir_at(trames_vfs::AT_FDCWD, b"/etc", 0o755)?;
        }
        vfs.import_reader(
            b"/etc/resolv.conf",
            &mut &b"nameserver 169.254.0.53\n"[..],
            false,
        )?;
    }
    for import in imports {
        let (guest, host) = import.split_once('=').ok_or("import must be GUEST=HOST")?;
        vfs.import_reader(guest.as_bytes(), &mut std::fs::File::open(host)?, true)?;
    }
    for (specification, readonly) in mounts {
        let (guest, host) = specification
            .split_once('=')
            .ok_or("mount must be GUEST=HOST")?;
        if !guest.starts_with('/') {
            return Err("mount target must be an absolute guest path".into());
        }
        super::files::mkdirs(&mut vfs, guest.as_bytes())?;
        vfs.mount_host(guest.as_bytes(), host, readonly)?;
    }
    if filesystem {
        super::files::run(&mut vfs, &argv)?;
        return Ok(0);
    }
    let bytes = if rootfs.is_some() || storage.is_some() {
        vfs.executable(path.as_bytes())?
    } else {
        let bytes = std::fs::read(path)?;
        vfs.import_executable(b"/program", bytes.clone())?;
        bytes
    };
    let server = if windows {
        Some(vfs.enable_wayland(b"/run/user/0")?)
    } else {
        None
    };
    if let (Some(bridge), Some(server)) = (&bridge, &server) {
        bridge.attach(server.clone());
    }
    let mut envp = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        "HOME=/root".into(),
        "LANG=C.UTF-8".into(),
        "TERM=xterm-256color".into(),
    ];
    if server.is_some() {
        envp.extend([
            "XDG_RUNTIME_DIR=/run/user/0".into(),
            "WAYLAND_DISPLAY=wayland-0".into(),
            "GDK_BACKEND=wayland".into(),
        ]);
    }
    let mut runtime = Runtime::load_with_options(&bytes, &argv, &envp, vfs, options)?;
    runtime.linux.trace = trace;
    // A browser's guest threads are scheduler jobs, not host threads. Keep
    // enough task slots for normal multiprocess pages without reserving RAM.
    runtime.linux.task_limit = 1024;
    runtime.cpu_count = cpus;
    trames_hvf::set_resident_limit(resident_limit);
    runtime.profile_syscalls = metrics;
    if pty {
        runtime.enable_terminal(Terminal::default())?;
    }
    let _signals = super::lifecycle::Signals::install()?;
    let _terminal = RawTerminal::enter(pty)?;
    let mut output = Output {
        host_memory_total: options
            .memory_limits
            .is_host_managed()
            .then(super::host_memory::physical_bytes)
            .transpose()?,
        deadline: (timeout != 0).then(|| Instant::now() + Duration::from_secs(timeout)),
        pressure_target: None,
        next_admin_check: Instant::now(),
        bridge,
        window: None,
    };
    super::lifecycle::ready();
    let result = runtime.run(&mut output, u64::MAX);
    if metrics {
        eprintln!(
            "trames mac parallel: peak={} chunks={} cpu_time_us={}",
            runtime.metrics.peak_parallel_cpus,
            runtime.metrics.worker_chunks,
            runtime.metrics.cpu_time.as_micros()
        );
        eprintln!(
            "trames mac cohorts: rounds={} control_checks={} services={} memory_us={} dispatch_us={} stop_us={}",
            runtime.metrics.parallel_rounds,
            runtime.metrics.persistent_control_checks,
            runtime.metrics.service_requests,
            runtime.metrics.parallel_memory_time.as_micros(),
            runtime.metrics.parallel_dispatch_time.as_micros(),
            runtime.metrics.parallel_stop_time.as_micros()
        );
        let mut returned: Vec<_> = runtime.metrics.cohort_return_syscalls.iter().collect();
        returned.sort_unstable_by_key(|(_, count)| std::cmp::Reverse(**count));
        eprintln!("trames mac cohort-return syscalls: {returned:?}");
        eprintln!(
            "trames mac cohort-return native: {:?}",
            runtime.metrics.cohort_return_native
        );
        eprintln!("trames mac paging: {:?}", trames_hvf::paging_stats());
        eprintln!(
            "trames mac vm phases us: {:?}",
            runtime
                .metrics
                .vm_checkpoint_time
                .map(|time| time.as_micros())
        );
        eprintln!(
            "trames mac vm quiescence bins: {:?}",
            runtime.metrics.vm_checkpoint_wait_bins
        );
        eprintln!(
            "trames mac vm batch: {:?}",
            runtime.metrics.vm_checkpoint_batch
        );

        eprintln!(
            "trames mac vm checkpoint calls: {:?}",
            runtime.metrics.vm_checkpoint_calls
        );
        eprintln!(
            "trames mac native: calls={} gpu_compiles={} gpu_wait_us={} upload_bytes={} download_bytes={} direct_bytes={} buffered_bytes={}",
            runtime.metrics.native.calls,
            runtime.metrics.gpu_compiles,
            runtime.metrics.gpu_wait_time.as_micros(),
            runtime.metrics.gpu_transfers.upload_bytes,
            runtime.metrics.gpu_transfers.download_bytes,
            runtime.metrics.gpu_transfers.direct_bytes,
            runtime.metrics.gpu_transfers.buffered_bytes
        );
        eprintln!(
            "trames mac shared GPU: maps={} mapped_bytes={}",
            runtime.metrics.gpu_transfers.shared_maps,
            runtime.metrics.gpu_transfers.shared_map_bytes
        );
        eprintln!(
            "trames mac coherent: boundaries={} ranges={} upload_scanned_bytes={} download_scanned_bytes={}",
            runtime.metrics.gpu_transfers.coherent_syncs,
            runtime.metrics.gpu_transfers.coherent_ranges,
            runtime.metrics.gpu_transfers.coherent_upload_scanned_bytes,
            runtime.metrics.gpu_transfers.coherent_download_scanned_bytes
        );
        eprintln!(
            "trames mac metrics: {{\"syscalls\":{},\"context_switches\":{},\"hardware_mapped_bytes\":{},\"guest_resident_bytes\":{},\"vm_checkpoint_groups\":{},\"vm_checkpoint_spaces\":{},\"backing_domain_checks\":{},\"backing_page_checks\":{},\"parallel_memory_us\":{},\"instruction_count\":null}}",
            runtime.metrics.syscalls,
            runtime.metrics.context_switches,
            runtime.engine.mapped_bytes(),
            runtime.memory.resident_bytes(),
            runtime.metrics.vm_checkpoint_groups,
            runtime.metrics.vm_checkpoint_spaces,
            runtime.metrics.parallel_snapshots.backing_domain_checks,
            runtime.metrics.parallel_snapshots.backing_page_checks,
            runtime.metrics.parallel_memory_time.as_micros()
        );
    }
    runtime.linux.vfs.sync()?;
    if super::lifecycle::stopping() || output.bridge.as_ref().is_some_and(|b| b.stopped()) {
        return Ok(super::lifecycle::exit_code());
    }
    result.map(|exit| exit.code).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessions_are_unlimited_unless_timeout_is_requested() {
        let parse = |args: &[&str]| {
            Command::parse(args.iter().map(|a| (*a).into()), false)
                .unwrap()
                .unwrap()
        };
        assert_eq!(parse(&["/bin/sh"]).timeout, 0);
        assert_eq!(parse(&["--timeout", "17", "/bin/sh"]).timeout, 17);
        assert_eq!(parse(&["--timeout", "0", "/bin/sh"]).timeout, 0);
    }
    #[test]
    fn headless_selection_uses_parsed_host_options_only() {
        let parse = |args: &[&str]| {
            Command::parse(args.iter().map(|s| (*s).to_owned()), false)
                .unwrap()
                .unwrap()
        };
        let headless = parse(&["--network", "off", "--no-windows", "--vulkan", "/bin/sh"]);
        assert!(!headless.windows && headless.vulkan);
        assert!(parse(&["--storage", "--no-windows", "/bin/sh"]).windows);
        assert!(parse(&["/bin/echo", "--no-windows"]).windows);
        assert!(parse(&["--", "/bin/echo", "--no-windows"]).windows);
        assert!(parse(&["/bin/sh"]).windows);
        assert!(Command::parse(
            ["--vulkan", "--unknown"].into_iter().map(str::to_owned),
            false
        )
        .is_err());
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn pressure_notification_only_latches_until_joined_control_reclaims() {
        unsafe extern "C" {
            fn trames_hvf_inject_pressure_for_test(bits: u32);
        }
        let mut memory = trames_memory::Memory::default();
        memory
            .map(
                0x10000,
                trames_memory::PAGE_SIZE,
                trames_memory::Permissions::RW,
            )
            .unwrap();
        memory
            .write_array(0x10000, 0xd4000001u32.to_le_bytes())
            .unwrap();
        memory
            .protect(
                0x10000,
                trames_memory::PAGE_SIZE,
                trames_memory::Permissions::RX,
            )
            .unwrap();
        let mut executor = trames_hvf::Executor::new().unwrap();
        let mut state = trames_execution::Aarch64State {
            pc: 0x10000,
            ..Default::default()
        };
        loop {
            if executor
                .run(&mut state, &mut memory, Duration::from_millis(2))
                .unwrap()
                .reason
                == trames_execution::ExitReason::Syscall
            {
                break;
            }
        }
        let before = trames_hvf::paging_stats();
        assert!(before.mapped_bytes > 0);
        let mut output = Output {
            host_memory_total: None,
            deadline: Some(Instant::now() + Duration::from_secs(5)),
            window: None,
            pressure_target: None,
            next_admin_check: Instant::now(),
            bridge: None,
        };
        let mut vfs = trames_vfs::Vfs::default();
        trames_hvf::take_memory_pressure().unwrap();
        // SAFETY: process-local metadata injection; no system-wide pressure.
        unsafe {
            trames_hvf_inject_pressure_for_test(4);
        }
        assert!(output.poll_control(&vfs).unwrap());
        assert_eq!(trames_hvf::paging_stats().mapped_bytes, before.mapped_bytes);
        assert!(output.control(&mut vfs).unwrap());
        assert_eq!(trames_hvf::paging_stats().mapped_bytes, 0);
        state.pc = 0x10000;
        loop {
            if executor
                .run(&mut state, &mut memory, Duration::from_millis(2))
                .unwrap()
                .reason
                == trames_execution::ExitReason::Syscall
            {
                break;
            }
        }
        assert!(trames_hvf::paging_stats().page_ins > before.page_ins);
    }
    #[test]
    fn memory_defaults_to_host_management_and_accepts_an_explicit_cap_or_auto_reset() {
        let parse = |args: &[&str]| {
            Command::parse(args.iter().map(|s| (*s).to_owned()), false)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            parse(&["/bin/sh"]).limit,
            MemoryLimits::host_managed().total
        );
        assert_eq!(
            parse(&["--memory-mib", "8192", "/bin/sh"]).limit,
            8usize << 30
        );
        assert_eq!(
            parse(&["--memory-mib", "8192", "--memory-mib", "auto", "/bin/sh"]).limit,
            MemoryLimits::host_managed().total
        );
    }
    #[test]
    fn exact_instruction_budget_is_rejected_before_creating_hardware() {
        let mut runtime = Runtime {
            memory_limits: MemoryLimits::default(),
            vulkan_display: Default::default(),
            cpu: Default::default(),
            engine: Default::default(),
            memory: Default::default(),
            linux: trames_linux::Linux::new(0),
            terminal: None,
            metrics: Default::default(),
            profile_syscalls: false,
            cpu_count: 1,
        };
        let error = runtime
            .run(&mut trames_linux::CapturedOutput::default(), 1)
            .unwrap_err();
        assert!(matches!(
            error,
            trames_session::RuntimeError::Configuration(_)
        ));
        assert_eq!(runtime.engine.mapped_bytes(), 0);
    }
}
