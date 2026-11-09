// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};
use trames_linux::{GuestOutput, OutputFd, TerminalEvent};
use trames_session::{MemoryLimits, RuntimeOptions, Terminal};
type Runtime = trames_session::Runtime<trames_hvf::kernel::Cpu>;

struct Output {
    source: Option<std::sync::Arc<dyn trames_vfs::ConsoleIo>>,
    exec: Option<super::exec::Server>,
    init: Option<super::systemd::Monitor>,
    console: Option<super::console::Console>,
    terminals: Option<trames_distro::terminals::NativeTerminals>,
    root_signal: Option<u32>,
    host_memory_total: Option<usize>,
    deadline: Option<Instant>,
    window: Option<(u16, u16)>,
    pressure: Option<trames_hvf::Reclaim>,
    next_stop_check: Instant,
    bridge: Option<std::sync::Arc<super::windows::Bridge>>,
}
impl Output {
    fn poll_init(&mut self) -> io::Result<()> {
        if Instant::now() < self.next_stop_check {
            return Ok(());
        }
        super::lifecycle::poll_stop();
        self.next_stop_check = Instant::now() + Duration::from_millis(100);
        if let Some(init) = &mut self.init {
            if let Some(signal) = init
                .control(super::lifecycle::stopping())
                .map_err(|e| io::Error::other(e.to_string()))?
            {
                self.root_signal = Some(signal);
            }
        }
        Ok(())
    }
    fn observe_pressure(&mut self) -> io::Result<()> {
        match trames_hvf::take_memory_pressure().map_err(io::Error::other)? {
            1 => self.pressure = None,
            level if level & 6 != 0 => {
                let mapped = trames_hvf::paging_stats().mapped_bytes;
                let target = if level & 4 != 0 {
                    mapped / 4
                } else {
                    mapped / 4 * 3
                };
                self.pressure = trames_hvf::reclaim_request(mapped - target);
            }
            _ => {}
        }
        if let Some(request) = &mut self.pressure {
            if request.progress().map_err(io::Error::other)? {
                self.pressure = None;
            }
        }
        Ok(())
    }
}
pub(crate) fn window_size() -> Option<(u16, u16)> {
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
    fn source_console(&mut self) -> std::sync::Arc<dyn trames_vfs::ConsoleIo> {
        if let Some(source) = &self.source {
            return source.clone();
        }
        let source: std::sync::Arc<dyn trames_vfs::ConsoleIo> = match &self.console {
            Some(console) => console.source(),
            None => std::sync::Arc::new(trames_vfs::NativeConsole::stdio()),
        };
        self.source = Some(source.clone());
        source
    }

    fn host_memory_source(
        &self,
    ) -> Option<std::sync::Arc<dyn Fn() -> trames_linux::HostMemoryInfo + Send + Sync>> {
        self.host_memory_total
            .map(|total| std::sync::Arc::new(move || super::host_memory::snapshot(total)) as _)
    }
    fn host_memory(&self) -> Option<trames_linux::HostMemoryInfo> {
        self.host_memory_total.map(super::host_memory::snapshot)
    }
    fn live_host_processes(&self) -> bool {
        true
    }
    fn poll_control(&mut self, vfs: &trames_vfs::Vfs) -> io::Result<bool> {
        self.observe_pressure()?;
        self.poll_init()?;
        if let Some(exec) = &mut self.exec {
            exec.poll()?;
            exec.control(vfs);
        }
        if let (Some(console), Some(terminals)) = (&mut self.console, &mut self.terminals) {
            console.poll()?;
            terminals.pump(&mut console.channels);
            console.flush();
        }
        Ok((super::lifecycle::stopping() && self.init.is_none())
            || self.bridge.as_ref().is_some_and(|b| b.stopped())
            || self.deadline.is_some_and(|d| Instant::now() >= d))
    }
    fn control(&mut self, vfs: &mut trames_vfs::Vfs) -> io::Result<bool> {
        self.poll_init()?;
        if let Some(exec) = &mut self.exec {
            exec.poll()?;
            exec.control(vfs);
        }
        if let (Some(console), Some(terminals)) = (&mut self.console, &mut self.terminals) {
            // The solo/cooperative driver calls this hook directly, without
            // the persistent workers' preceding poll_control opportunity.
            console.poll()?;
            terminals.pump(&mut console.channels);
            console.flush();
        }
        if (super::lifecycle::stopping() && self.init.is_none())
            || self.bridge.as_ref().is_some_and(|b| b.stopped())
            || self.deadline.is_some_and(|d| Instant::now() >= d)
        {
            return Ok(false);
        }
        self.observe_pressure()?;
        Ok(true)
    }
    fn host_spawn(&mut self) -> Option<trames_linux::HostSpawn> {
        self.exec.as_mut()?.spawn()
    }
    fn root_signal(&mut self) -> Option<u32> {
        self.root_signal.take()
    }
    fn write(&mut self, fd: OutputFd, bytes: &[u8]) -> io::Result<usize> {
        if let Some(console) = &mut self.console {
            return console.write(bytes);
        }
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
        if self.console.is_some() {
            return Ok(false);
        }
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
        if self.console.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
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
        if self.console.is_some() {
            return None;
        }
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
pub(crate) struct RawTerminal(Option<libc::termios>);
impl RawTerminal {
    pub(crate) fn enter(enabled: bool) -> io::Result<Self> {
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
    audio: bool,
    audio_input: bool,
    pty: bool,
    init: super::systemd::Init,
    console: Option<std::path::PathBuf>,
    terminal_command: Vec<String>,
    argv: Vec<String>,
}
impl Command {
    #[cfg(test)]
    pub fn parse(
        arguments: impl Iterator<Item = String>,
        pty: bool,
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        Self::parse_with_default(arguments, pty, &["/bin/sh".into(), "-l".into()])
    }
    pub fn parse_with_default(
        arguments: impl Iterator<Item = String>,
        pty: bool,
        default_command: &[String],
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let mut arguments = arguments.peekable();
        let mut init = super::systemd::Init::Shell;
        let mut console = None;
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
        let mut audio = false;
        let mut audio_input = false;
        let mut pty = pty;
        while arguments.peek().is_some_and(|a| a.starts_with("--")) {
            match arguments.next().unwrap().as_str() {
                "--console-socket" => {
                    console = Some(std::path::PathBuf::from(
                        arguments.next().ok_or("missing console socket")?,
                    ))
                }
                "--init" => {
                    init =
                        super::systemd::Init::parse(&arguments.next().ok_or("missing init mode")?)?
                }
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
                "--audio" => audio = true,
                "--audio-input" => {
                    audio = true;
                    audio_input = true;
                }
                "--trace" => trace = true,
                "--metrics" => metrics = true,
                "--pty" => pty = true,
                "--no-pty" => pty = false,
                "--" => break,
                "--help" => {
                    println!(
                        "trames-mac-run [--init shell|systemd] [--rootfs TAR_OR_DIRECTORY] [--storage DIR] [--exec-file GUEST=HOST] [--mount GUEST=HOST] [--mount-ro GUEST=HOST] [--fs ls|cat|get|put ...] [--network off|loopback|outbound|full] [--memory-mib auto|N] [--cpus N] [--resident-mib N] [--timeout SECONDS] [--pty|--no-pty] [--no-windows] [--vulkan] [--video] [--audio] [--audio-input] [--trace] [--metrics] [ELF [ARG...]]\nLinux userspace with hardware ARM64 execution; defaults to host-managed memory and /bin/sh -l with a rootfs."
                    );
                    return Ok(None);
                }
                _ => return Err("unknown option".into()),
            }
        }
        let mut argv: Vec<String> = arguments.collect();
        if init == super::systemd::Init::Systemd {
            if filesystem || !argv.is_empty() {
                return Err(
                    "systemd boot takes no Linux command; use --init shell for offline maintenance"
                        .into(),
                );
            }
            pty = true;
            argv = vec![
                "/usr/lib/systemd/systemd".into(),
                "--system".into(),
                "--unit=multi-user.target".into(),
            ];
        }
        if argv.is_empty() && !filesystem && (rootfs.is_some() || storage.is_some()) {
            argv = default_command.to_vec();
        }
        if console.is_some() {
            pty = true;
        }
        Ok(Some(Self {
            terminal_command: default_command.to_vec(),
            console,
            init,
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
            audio,
            audio_input,
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
        terminal_command,
        console,
        init,
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
        audio,
        audio_input,
        pty,
        mut argv,
    } = command;
    let launcher = trames_distro::terminals::launcher(&terminal_command);
    if console.is_some() && init == super::systemd::Init::Shell {
        argv = vec!["/bin/sh".into(), "-c".into(), launcher.clone()];
    }
    let path = argv
        .first()
        .ok_or("an ELF program or --rootfs is required; use --help")?;
    let options = RuntimeOptions {
        vulkan_accelerator: vulkan,
        video_accelerator: video,
        audio_output: audio,
        audio_input,
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
        if vfs.capacity().nodes_used <= 3 {
            if storage.is_some() {
                vfs.populate_staged(|vfs| {
                    super::rootfs::install(std::path::Path::new(archive), vfs, None)
                })?;
            } else {
                super::rootfs::install(std::path::Path::new(archive), &mut vfs, None)?;
            }
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
    let init = if init == super::systemd::Init::Systemd {
        Some(super::systemd::prepare(&mut vfs)?)
    } else {
        None
    };
    let bytes = if rootfs.is_some() || storage.is_some() {
        vfs.executable(path.as_bytes())?
    } else {
        let bytes = std::fs::read(path)?;
        vfs.import_executable(b"/program", bytes.clone())?;
        bytes
    };
    let terminals = if console.is_some() {
        Some(
            if init.is_some() {
                trames_distro::terminals::NativeTerminals::for_systemd_launcher(&mut vfs, &launcher)
            } else {
                trames_distro::terminals::NativeTerminals::new(&mut vfs)
            }
            .map_err(io::Error::other)?,
        )
    } else {
        None
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
    if init.is_some() {
        envp.extend([
            "container=trames".into(),
            "NOTIFY_SOCKET=/run/host/notify".into(),
        ]);
        if server.is_some() {
            super::systemd::file(&mut vfs, b"/run/systemd/system/console-getty.service.d/60-wayland.conf", b"[Service]\nEnvironment=XDG_RUNTIME_DIR=/run/user/0 WAYLAND_DISPLAY=wayland-0 GDK_BACKEND=wayland\n")?;
        }
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
    let mut output = Output {
        source: None,
        exec: console
            .as_deref()
            .map(|path| super::exec::Server::bind(path, envp.clone()))
            .transpose()?,
        terminals,
        console: console
            .as_deref()
            .map(super::console::Console::bind)
            .transpose()?,
        init,
        root_signal: None,
        host_memory_total: options
            .memory_limits
            .is_host_managed()
            .then(super::host_memory::physical_bytes)
            .transpose()?,
        deadline: (timeout != 0).then(|| Instant::now() + Duration::from_secs(timeout)),
        pressure: None,
        next_stop_check: Instant::now(),
        bridge,
        window: None,
    };
    // Host diagnostics use the original terminal's newline processing. Guest
    // output keeps its own termios rules while the host transport is raw.
    super::lifecycle::ready();
    let terminal = RawTerminal::enter(pty)?;
    let result = runtime.run(&mut output, u64::MAX);
    drop(terminal);
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
        // Existing counters only: enabling diagnostics adds no production-path work.
        let m = &runtime.metrics;
        eprintln!(
            "trames mac execution paths: {}",
            serde_json::json!({
                "syscalls": m.syscalls,
                "worker_syscalls": m.worker_syscalls,
                "worker_direct_syscalls": m.worker_direct_syscalls,
                "worker_file_syscalls": m.worker_file_syscalls,
                "worker_socket_syscalls": m.worker_socket_syscalls,
                "worker_fd_syscalls": m.worker_fd_syscalls,
                "worker_poll_syscalls": m.worker_poll_syscalls,
                "worker_uring_syscalls": m.worker_uring_syscalls,
                "worker_futex_waits": m.worker_futex_waits,
                "worker_futex_wakes": m.worker_futex_wakes,
                "worker_task_resumes": m.worker_task_resumes,
                "worker_gpu_calls": m.worker_gpu_calls,
                "worker_gpu_waits": m.worker_gpu_waits,
                "service_requests": m.service_requests,
                "serviced_syscalls": m.serviced_syscalls,
                "vm_local_syscalls": m.vm_local_syscalls,
                "cohort_rounds": m.parallel_rounds,
                "cohort_stop_us": m.parallel_stop_time.as_micros(),
                "service_caller_wait_us": m.service_wait_time.as_micros(),
            })
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
    fn systemd_requires_explicit_shell_override_for_maintenance_commands() {
        let parse = |args: &[&str]| Command::parse(args.iter().map(|a| (*a).into()), false);
        let boot = parse(&["--storage", "/store", "--init", "systemd"])
            .unwrap()
            .unwrap();
        assert!(boot.pty && boot.init == super::super::systemd::Init::Systemd);
        assert_eq!(boot.argv[0], "/usr/lib/systemd/systemd");
        assert!(parse(&["--init", "systemd", "/bin/true"]).is_err());
        assert!(parse(&["--init", "systemd", "--init", "shell", "/bin/true"]).is_ok());
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
    fn pressure_notification_reclaims_without_join_and_restores_original_bytes() {
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
            source: None,
            exec: None,
            console: None,
            terminals: None,
            init: None,
            root_signal: None,
            host_memory_total: None,
            deadline: Some(Instant::now() + Duration::from_secs(5)),
            window: None,
            pressure: None,
            next_stop_check: Instant::now(),
            bridge: None,
        };
        let mut vfs = trames_vfs::Vfs::default();
        trames_hvf::take_memory_pressure().unwrap();
        // SAFETY: process-local metadata injection; no system-wide pressure.
        unsafe {
            trames_hvf_inject_pressure_for_test(4);
        }
        assert!(!output.poll_control(&vfs).unwrap());
        assert_eq!(trames_hvf::paging_stats().mapped_bytes, 0);
        assert!(output.pressure.is_none());
        assert!(output.control(&mut vfs).unwrap());
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
    fn threaded_image(cpus: usize, tail: &[u32]) -> Vec<u8> {
        let mut words = vec![0xd2800000 | (((cpus - 1) as u32) << 5) | 19];
        if cpus > 1 {
            words.extend_from_slice(&[
                0xd281e000,
                0xf2a00020,
                0x910003e1,
                0xcb133021,
                0xd2800002,
                0xd2800003,
                0xd2800004,
                0xd2801b88,
                0xd4000001,
                0xb4000060,
                0xd1000673,
                0xb5000000 | (((-11i32 as u32) & 0x7ffff) << 5) | 19,
            ]);
        }
        words.extend_from_slice(tail);
        let mut bytes = vec![0; 256 + 4 * words.len()];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        for (at, value) in [(16, 2u16), (18, 183), (52, 64), (54, 56), (56, 1)] {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in [(20, 1u32), (64, 1), (68, 5)] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let length = bytes.len() as u64;
        for (at, value) in [
            (24, 0x400100u64),
            (32, 64),
            (80, 0x400000),
            (96, length),
            (104, length),
            (112, 4096),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (i, word) in words.into_iter().enumerate() {
            bytes[256 + i * 4..260 + i * 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn descriptor_discovery_controls_use_original_native_workers_without_ioctl_joins() {
        use trames_execution::Registers;
        let imm = |words: &mut Vec<u32>, reg: u32, n: u32| {
            words.push(0xd2800000 | ((n & 0xffff) << 5) | reg);
            if n > 0xffff {
                words.push(0xf2a00000 | ((n >> 16) << 5) | reg);
            }
        };
        for cpus in [4, 12] {
            let mut tail = Vec::new();
            let mut branches = Vec::new();
            for (reg, request, pointer, expected) in [
                (25, 0x1234, 0, u32::MAX),
                (26, 0x8933, 0x50000, 0),
                (27, 0xc0406400, 0x51000, 0),
                (28, 0xb703, 0, 0x40000000),
            ] {
                tail.push(0xaa0003e0 | (reg << 16));
                imm(&mut tail, 1, request);
                imm(&mut tail, 2, pointer);
                imm(&mut tail, 8, 29);
                tail.push(0xd4000001);
                if expected == u32::MAX {
                    tail.push(0xb100641f);
                } else {
                    imm(&mut tail, 3, expected);
                    tail.push(0xeb03001f);
                }
                branches.push(tail.len());
                tail.push(0);
            }
            imm(&mut tail, 0, 0);
            imm(&mut tail, 8, 93);
            tail.extend([0xd4000001, 0x14000000]);
            let failed = tail.len();
            imm(&mut tail, 0, 91);
            imm(&mut tail, 8, 94);
            tail.extend([0xd4000001, 0x14000000]);
            for at in branches {
                tail[at] = 0x54000001 | ((((failed - at) as u32) & 0x7ffff) << 5);
            }
            let image = threaded_image(cpus, &tail);
            let mut runtime = Runtime::load(&image, &["native-ioctl".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime.linux.vfs.enable_video_device();
            runtime
                .linux
                .vfs
                .configure_network(trames_vfs::NetworkMode::Loopback)
                .unwrap();
            runtime.linux.vfs.set_runtime_info(
                1,
                trames_vfs::RuntimeInfo {
                    processes: vec![trames_vfs::ProcessInfo {
                        pid: 1,
                        tgid: 1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
            let pipe = runtime.linux.vfs.pipe(0).unwrap()[0];
            let socket = runtime.linux.vfs.socket(2, 2, 0).unwrap();
            let render = runtime
                .linux
                .vfs
                .open_at(-100, b"/dev/dri/renderD128", 2, 0)
                .unwrap();
            let namespace = runtime
                .linux
                .vfs
                .open_at(-100, b"/proc/self/ns/net", 0, 0)
                .unwrap();
            for (reg, fd) in [(25, pipe), (26, socket), (27, render), (28, namespace)] {
                runtime.cpu.set_x(reg, fd as u64);
            }
            runtime
                .memory
                .map(0x50000, 3 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            runtime.memory.write(0x50000, b"lo\0").unwrap();
            for offset in [16u64, 32, 48] {
                runtime
                    .memory
                    .write_array(0x51000 + offset, 64u64.to_le_bytes())
                    .unwrap();
                runtime
                    .memory
                    .write_array(0x51008 + offset, (0x52000 + offset * 4).to_le_bytes())
                    .unwrap();
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware ioctl CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            assert_eq!(
                runtime.memory.read_array::<4>(0x50010).unwrap(),
                1u32.to_le_bytes()
            );
            assert_eq!(
                runtime.memory.read_array::<4>(0x51000).unwrap(),
                1u32.to_le_bytes()
            );
            assert_eq!(runtime.metrics.cohort_return_syscalls.get(&29), None);
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn prctl_names_and_sme_configuration_complete_on_original_native_workers() {
        use trames_execution::Registers;
        for cpus in [4, 12] {
            let code: &[u32] = match cpus {
                4 => &[
                    0xd2a000b5, 0x8b1316b5, 0xd2820014, 0xf2a000b4, 0x8b131694, 0xd2840016,
                    0xf2a000b6, 0xd28001e0, 0xaa1503e1, 0xd28014e8, 0xd4000001, 0xb5000460,
                    0x885ffec9, 0x11000529, 0x880afec9, 0x35ffffaa, 0x88dffec9, 0x7100113f,
                    0x54ffffc3, 0xd2800200, 0xaa1403e1, 0xd4000001, 0xb5000300, 0xd28000a0,
                    0xd2800001, 0xd4000001, 0xb100581f, 0x54000261, 0xd2800800, 0xd4000001,
                    0xeb1b001f, 0x540001e1, 0xd28007e0, 0xd2800801, 0xf2a00041, 0xd4000001,
                    0xeb1c001f, 0x54000121, 0xd2800800, 0xd4000001, 0xeb1c001f, 0x540000a1,
                    0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000, 0xd2800b60, 0xd2800bc8,
                    0xd4000001, 0x14000000,
                ],
                12 => &[
                    0xd2a000b5, 0x8b1316b5, 0xd2820014, 0xf2a000b4, 0x8b131694, 0xd2840016,
                    0xf2a000b6, 0xd28001e0, 0xaa1503e1, 0xd28014e8, 0xd4000001, 0xb5000460,
                    0x885ffec9, 0x11000529, 0x880afec9, 0x35ffffaa, 0x88dffec9, 0x7100313f,
                    0x54ffffc3, 0xd2800200, 0xaa1403e1, 0xd4000001, 0xb5000300, 0xd28000a0,
                    0xd2800001, 0xd4000001, 0xb100581f, 0x54000261, 0xd2800800, 0xd4000001,
                    0xeb1b001f, 0x540001e1, 0xd28007e0, 0xd2800801, 0xf2a00041, 0xd4000001,
                    0xeb1c001f, 0x54000121, 0xd2800800, 0xd4000001, 0xeb1c001f, 0x540000a1,
                    0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000, 0xd2800b60, 0xd2800bc8,
                    0xd4000001, 0x14000000,
                ],
                _ => unreachable!(),
            };
            let image = threaded_image(cpus, code);
            let mut runtime = Runtime::load(&image, &["native-prctl".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let sme = trames_hvf::features().hwcap2 & (1 << 23) != 0;
            runtime
                .cpu
                .set_x(27, if sme { 64 } else { 0u64.wrapping_sub(22) });
            runtime.cpu.set_x(
                28,
                if sme {
                    64 | (1 << 17)
                } else {
                    0u64.wrapping_sub(22)
                },
            );
            runtime
                .memory
                .map(0x50000, 3 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            for id in 0..cpus {
                runtime
                    .memory
                    .write(
                        0x50000 + id as u64 * 32,
                        format!("native-{id:02}\0").as_bytes(),
                    )
                    .unwrap();
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware prctl CPUs={cpus} SME={sme} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                let actual = runtime
                    .memory
                    .read_array::<16>(0x51000 + id as u64 * 32)
                    .unwrap();
                let expected = format!("native-{id:02}\0");
                assert_eq!(&actual[..expected.len()], expected.as_bytes());
            }
            assert_eq!(runtime.metrics.cohort_return_syscalls.get(&167), None);
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn time_observations_and_vdso_realtime_complete_on_original_native_workers() {
        use trames_execution::Registers;
        let code: &[u32] = &[
            0xd2a000b4, 0x8b132294, 0xaa1403e0, 0x91004281, 0xd2801528, 0xd4000001, 0xb5000560,
            0xf9400289, 0xb4000529, 0xf9400689, 0xd288480a, 0xf2a001ea, 0xeb0a013f, 0x54000482,
            0xf9400a89, 0xb5000449, 0xaa1403e0, 0xd2800001, 0xd63f0340, 0xb50003c0, 0xd2800000,
            0xd2800001, 0xd2801528, 0xd4000001, 0xb5000320, 0xd2800000, 0x91008281, 0xd2800e28,
            0xd4000001, 0xb5000280, 0xd2800020, 0x9100c281, 0xd4000001, 0xb5000200, 0xd2800020,
            0xd2800001, 0xd2800e48, 0xd4000001, 0xb5000160, 0x91010280, 0xd2800021, 0xd2801528,
            0xd4000001, 0xb100381f, 0x540000a1, 0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000,
            0xd2800b60, 0xd2800bc8, 0xd4000001, 0x14000000,
        ];
        for cpus in [4, 12] {
            let image = threaded_image(cpus, code);
            let mut runtime = Runtime::load(&image, &["native-time".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 4096, trames_memory::Permissions::RW)
                .unwrap();
            // Resolve the actual versioned vDSO export, rather than a test stub
            // or a copied instruction offset. This exercises libc's SVC route.
            let base = runtime
                .linux
                .auxiliary
                .chunks_exact(16)
                .find(|a| {
                    u64::from_le_bytes(a[..8].try_into().unwrap()) == trames_linux::vdso::AUXV
                })
                .map(|a| u64::from_le_bytes(a[8..].try_into().unwrap()))
                .unwrap();
            let vdso = trames_linux::vdso::image();
            let u64_at =
                |at: usize| u64::from_le_bytes(vdso[at..at + 8].try_into().unwrap()) as usize;
            let u32_at =
                |at: usize| u32::from_le_bytes(vdso[at..at + 4].try_into().unwrap()) as usize;
            let ph = u64_at(32);
            let dynamic = (0..u16::from_le_bytes(vdso[56..58].try_into().unwrap()) as usize)
                .map(|i| ph + i * 56)
                .find(|&at| u32_at(at) == 2)
                .unwrap();
            let offset = u64_at(dynamic + 8);
            let entries = &vdso[offset..offset + u64_at(dynamic + 32)];
            let tag = |key: u64| {
                entries
                    .chunks_exact(16)
                    .find(|d| u64::from_le_bytes(d[..8].try_into().unwrap()) == key)
                    .map(|d| u64::from_le_bytes(d[8..].try_into().unwrap()) as usize)
                    .unwrap()
            };
            let (symbols, strings, count) = (tag(6), tag(5), u32_at(tag(4) + 4));
            let symbol = (1..count)
                .map(|i| symbols + i * 24)
                .find(|&s| {
                    vdso[strings + u32_at(s)..]
                        .split(|&b| b == 0)
                        .next()
                        .unwrap()
                        == b"__kernel_gettimeofday"
                })
                .unwrap();
            runtime.cpu.set_x(26, base + u64_at(symbol + 8) as u64);
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware time CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                let at = 0x50000 + id as u64 * 256;
                for delta in [0, 32] {
                    assert!(
                        u64::from_le_bytes(runtime.memory.read_array::<8>(at + delta).unwrap())
                            > 1_700_000_000
                    );
                }
                assert_eq!(runtime.memory.read_array::<8>(at + 16).unwrap(), [0; 8]);
                assert_eq!(runtime.memory.read_array::<16>(at + 64).unwrap(), [0; 16]);
            }
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn published_global_metadata_and_uring_complete_on_original_native_workers() {
        use trames_execution::Registers;
        let code: &[u32] = &[
            0xd2a000b4, 0x8b132294, 0xd2820015, 0xf2a000b5, 0xaa1903e0, 0xaa1403e1, 0xd2800102,
            0xd2800003, 0xd2800868, 0xd4000001, 0xf100201f, 0x540001e1, 0xb8737aa0, 0xd2800021,
            0xd2800022, 0xd2800023, 0xd2800004, 0xd2800005, 0xd2803548, 0xd4000001, 0xf100041f,
            0x540000a1, 0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000, 0xd2800b60, 0xd2800bc8,
            0xd4000001, 0x14000000,
        ];
        for cpus in [4, 12] {
            let image = threaded_image(cpus, code);
            let mut runtime = Runtime::load(&image, &["native-published".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime.linux.vfs.set_runtime_info(1, Default::default());
            let file = runtime
                .linux
                .vfs
                .open_at(-100, b"/proc/version", 0, 0)
                .unwrap();
            runtime.cpu.set_x(25, file as u64);
            runtime
                .memory
                .map(0x50000, 2 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            let mut rings = Vec::new();
            for id in 0..cpus {
                let (fd, ring) = runtime.linux.vfs.uring_create(4, 8).unwrap();
                let rb = 0x100000 + id as u64 * 0x10000;
                let sq = 0x200000 + id as u64 * 0x10000;
                for (at, offset, len) in
                    [(rb, 0, ring.ring_bytes), (sq, 0x10000000, ring.sqe_bytes)]
                {
                    runtime
                        .memory
                        .map_backed(
                            at,
                            runtime
                                .linux
                                .vfs
                                .mapping_pages(fd, offset, len, true, true)
                                .unwrap(),
                            true,
                            trames_memory::Permissions::RW,
                            false,
                        )
                        .unwrap();
                }
                runtime
                    .memory
                    .write_array(0x51000 + id as u64 * 4, fd.to_le_bytes())
                    .unwrap();
                let mut entry = [0u8; 64];
                entry[0] = 22;
                entry[4..8].copy_from_slice(&file.to_le_bytes());
                entry[16..24].copy_from_slice(&(0x50010 + id as u64 * 256).to_le_bytes());
                entry[24..28].copy_from_slice(&8u32.to_le_bytes());
                entry[32..40].copy_from_slice(&(id as u64 + 1).to_le_bytes());
                runtime.memory.write(sq, &entry).unwrap();
                runtime
                    .memory
                    .write_array(rb + 4, 1u32.to_le_bytes())
                    .unwrap();
                rings.push(ring);
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware published CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for (id, ring) in rings.iter().enumerate() {
                let at = 0x50000 + id as u64 * 256;
                assert_eq!(runtime.memory.read_array::<8>(at).unwrap(), *b"Linux ve");
                assert_eq!(
                    runtime.memory.read_array::<8>(at + 16).unwrap(),
                    *b"Linux ve"
                );
                assert_eq!(ring.cq_ready(), 1);
                assert_eq!(
                    ring.read_queue::<8>(trames_vfs::URING_CQES),
                    (id as u64 + 1).to_le_bytes()
                );
                assert_eq!(
                    ring.read_queue::<4>(trames_vfs::URING_CQES + 8),
                    8u32.to_le_bytes()
                );
            }
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn retained_original_uring_bodies_complete_without_a_join_on_native_workers() {
        use trames_execution::Registers;
        let code: &[u32] = &[
            0xd2a000b4, 0xb5000293, 0xaa1a03e0, 0x91002281, 0xd2800102, 0xd2800808, 0xd4000001,
            0xf100201f, 0x54000261, 0xaa1903e0, 0xd2800001, 0xd2800042, 0xd2800023, 0xd2800004,
            0xd2800005, 0xd2803548, 0xd4000001, 0xb5000140, 0x52800021, 0x889ffe81, 0x14000003,
            0x88dffe80, 0x34ffffe0, 0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000, 0xd2800b60,
            0xd2800bc8, 0xd4000001, 0x14000000,
        ];
        for cpus in [4, 12] {
            let image = threaded_image(cpus, code);
            let mut runtime =
                Runtime::load(&image, &["native-original-uring".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime.linux.vfs.set_runtime_info(1, Default::default());
            struct Unavailable;
            impl trames_vfs::ProcSource for Unavailable {
                fn available(&self, _: trames_vfs::ProcRefresh) -> bool {
                    false
                }
                fn observe(&self, _: trames_vfs::ProcQuery<'_>) -> trames_vfs::RuntimeInfo {
                    panic!("no source provider in the original embedding")
                }
            }
            runtime
                .linux
                .vfs
                .set_proc_source(std::sync::Arc::new(Unavailable));
            let file = runtime
                .linux
                .vfs
                .open_at(-100, b"/proc/version", 0, 0)
                .unwrap();
            let gate = runtime.linux.vfs.eventfd(0, 0).unwrap();
            let (fd, ring) = runtime.linux.vfs.uring_create(4, 8).unwrap();
            runtime.cpu.set_x(25, fd as u64);
            runtime.cpu.set_x(26, gate as u64);
            runtime
                .memory
                .map(0x50000, 4096, trames_memory::Permissions::RW)
                .unwrap();
            runtime
                .memory
                .write_array(0x50008, 1u64.to_le_bytes())
                .unwrap();
            for (at, offset, len) in [
                (0x100000, 0, ring.ring_bytes),
                (0x200000, 0x10000000, ring.sqe_bytes),
            ] {
                runtime
                    .memory
                    .map_backed(
                        at,
                        runtime
                            .linux
                            .vfs
                            .mapping_pages(fd, offset, len, true, true)
                            .unwrap(),
                        true,
                        trames_memory::Permissions::RW,
                        false,
                    )
                    .unwrap();
            }
            for (id, original, at) in [(0u64, gate, 0x50020u64), (1, file, 0x50010)] {
                let mut entry = [0u8; 64];
                entry[0] = 22;
                entry[1] = if id == 0 { 4 } else { 0 };
                entry[4..8].copy_from_slice(&original.to_le_bytes());
                entry[16..24].copy_from_slice(&at.to_le_bytes());
                entry[24..28].copy_from_slice(&8u32.to_le_bytes());
                entry[32..40].copy_from_slice(&(id + 1).to_le_bytes());
                runtime.memory.write(0x200000 + id * 64, &entry).unwrap();
            }
            runtime
                .memory
                .write_array(0x100004, 2u32.to_le_bytes())
                .unwrap();
            runtime
                .memory
                .write_array(
                    0x100000 + trames_vfs::URING_SQ_ARRAY as u64 + 4,
                    1u32.to_le_bytes(),
                )
                .unwrap();
            // Import before the live Kernel installs its provider. The original
            // accepted context and OFD survive that handoff, FD close and reuse.
            assert_eq!(
                runtime
                    .linux
                    .uring_enter([fd as u64, 2, 0, 0, 0, 0], &mut runtime.memory, None, 1),
                Ok((2, true))
            );
            assert_eq!(ring.cq_ready(), 0);
            runtime.linux.vfs.close(file).unwrap();
            assert_eq!(runtime.linux.vfs.eventfd(99, 0).unwrap(), file);
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware original-uring CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            assert_eq!(
                runtime.memory.read_array::<4>(0x50000).unwrap(),
                1u32.to_le_bytes()
            );
            assert_eq!(
                runtime.memory.read_array::<8>(0x50010).unwrap(),
                *b"Linux ve"
            );
            assert_eq!(
                runtime.memory.read_array::<8>(0x50020).unwrap(),
                1u64.to_le_bytes()
            );
            assert_eq!(ring.cq_ready(), 2);
            for id in 0..2 {
                assert_eq!(
                    ring.read_queue::<8>(trames_vfs::URING_CQES + id * 16),
                    (id as u64 + 1).to_le_bytes()
                );
                assert_eq!(
                    ring.read_queue::<4>(trames_vfs::URING_CQES + id * 16 + 8),
                    8u32.to_le_bytes()
                );
            }
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn caught_native_cpu_faults_and_signal_frames_complete_on_original_workers() {
        let code: &[u32] = &[
            0xd2a000b4, 0x8b131e94, 0xd2800160, 0xd2820001, 0xf2a000a1, 0xd2800002, 0xd2800103,
            0xd28010c8, 0xd4000001, 0xb5000140, 0xd2a01209, 0x8b133129, 0xf9400120, 0x52800021,
            0xb9001281, 0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000, 0xd2800b60, 0xd2800bc8,
            0xd4000001, 0x14000000, 0xb9000280, 0xb940082a, 0xb900068a, 0xf940082a, 0xf900068a,
            0xf940dc4a, 0x9100114a, 0xf900dc4a, 0xd65f03c0, 0xd2801168, 0xd4000001, 0x14000000,
        ];
        for cpus in [4, 12] {
            let mut image = threaded_image(cpus, code);
            let tail_offset = image.len() - code.len() * 4;
            // Signal frames exceed one 4 KiB page. Each thread needs its own
            // 16 KiB stack interval while its frame/SME records are live.
            let mut patched = false;
            for instruction in image[256..tail_offset].chunks_exact_mut(4) {
                if instruction == 0xcb133021u32.to_le_bytes() {
                    instruction.copy_from_slice(&0xcb133821u32.to_le_bytes());
                    patched = true;
                }
            }
            assert!(patched);
            let handler = 0x400000 + tail_offset as u64 + 0x5c;
            let restorer = 0x400000 + tail_offset as u64 + 0x80;
            let mut runtime = Runtime::load(&image, &["native-source-fault".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 8192, trames_memory::Permissions::RW)
                .unwrap();
            let action: Vec<u8> = [handler, 0x04000004, restorer, 0]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect();
            runtime.memory.write(0x51000, &action).unwrap();
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware source-fault CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                let at = 0x50000 + id as u64 * 128;
                assert_eq!(
                    runtime.memory.read_array::<4>(at).unwrap(),
                    11u32.to_le_bytes()
                );
                assert_eq!(
                    runtime.memory.read_array::<4>(at + 4).unwrap(),
                    1u32.to_le_bytes()
                );
                assert_eq!(
                    runtime.memory.read_u64(at + 8).unwrap(),
                    0x900000 + id as u64 * 4096
                );
                assert_eq!(
                    runtime.memory.read_array::<4>(at + 16).unwrap(),
                    1u32.to_le_bytes()
                );
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert!(runtime.metrics.cohort_return_native.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn original_fd_status_and_memfd_seals_complete_without_joined_native_dispatch() {
        use trames_execution::Registers;
        let imm = |code: &mut Vec<u32>, register: u32, value: u64| {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | register);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | register);
                }
            }
        };
        for cpus in [4, 12] {
            let noatime = trames_vfs::O_NOATIME as u64;
            // x25/x26/x27/x28 retain original status/foreign/busy/path FDs.
            // x21 loads this actual thread's memfd from the original table.
            let calls: [(u32, u64, u64, i64); 18] = [
                (25, 4, noatime, 0),
                (26, 4, noatime, -1),
                (25, 4, 0x80000000, -22),
                (28, 4, 0, -9),
                (25, 12, u64::MAX, -22),
                (28, 12, u64::MAX, -9),
                (21, 1034, 0, 0),
                (21, 1033, 16, -22),
                (21, 1033, 6, 0),
                (21, 1034, 0, 6),
                (27, 1033, 8, -16),
                (21, 1033, 8, 0),
                (21, 1034, 0, 14),
                (21, 1033, 1, 0),
                (21, 1034, 0, 15),
                (21, 1033, 0, -1),
                (25, 3, 0, (noatime | 2) as i64),
                (26, 1034, 0, -22),
            ];
            let mut code = vec![0xd2a000b4, 0x8b132294]; // x20=0x50000+thread*256
            imm(&mut code, 22, 0x52000);
            code.push(0xf8737ad5); // ldr x21,[x22,x19,lsl #3]
            for (i, (fd, command, argument, _)) in calls.iter().enumerate() {
                code.push(0xaa0003e0 | (*fd << 16));
                imm(&mut code, 1, *command);
                imm(&mut code, 2, *argument);
                imm(&mut code, 8, 25);
                code.extend([0xd4000001, 0xf9000280 | ((i as u32) << 10)]);
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime =
                Runtime::load(&threaded_image(cpus, &code), &["native-fcntl".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 12288, trames_memory::Permissions::RW)
                .unwrap();
            let v = &mut runtime.linux.vfs;
            v.import_file(b"/owned", b"original".to_vec()).unwrap();
            v.import_file(b"/foreign", Vec::new()).unwrap();
            let owned = v.open_at(-100, b"/owned", 2, 0).unwrap();
            v.chown_fd(owned, 1000, 1000).unwrap();
            let foreign = v.open_at(-100, b"/foreign", 2, 0).unwrap();
            let busy = v.memfd_create(b"busy", 2).unwrap();
            v.truncate_fd(busy, 4096).unwrap();
            runtime
                .memory
                .map_backed(
                    0x70000,
                    v.mapping_pages(busy, 0, 4096, true, true).unwrap(),
                    true,
                    trames_memory::Permissions::RW,
                    false,
                )
                .unwrap();
            let path = v.open_at(-100, b"/", trames_vfs::O_PATH, 0).unwrap();
            let mut memfds = Vec::new();
            for id in 0..cpus {
                let fd = v.memfd_create(b"thread-seal", 2).unwrap();
                v.write(fd, b"payload").unwrap();
                runtime
                    .memory
                    .write(0x52000 + id as u64 * 8, &(fd as u64).to_le_bytes())
                    .unwrap();
                runtime
                    .memory
                    .map_backed(
                        0x60000 + id as u64 * 4096,
                        v.mapping_pages(fd, 0, 4096, false, false).unwrap(),
                        false,
                        trames_memory::Permissions::R,
                        false,
                    )
                    .unwrap();
                memfds.push(fd);
            }
            v.credentials = trames_vfs::Credentials::user(1000, 1000);
            for (register, fd) in [(25, owned), (26, foreign), (27, busy), (28, path)] {
                runtime.cpu.set_x(register, fd as u64);
            }
            // Keep original inode observations after the guest closes its Files.
            let observer = runtime.linux.vfs.clone_with_files(false);
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware fcntl CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for (id, fd) in memfds.iter().enumerate() {
                for (i, (_, command, _, expected)) in calls.iter().enumerate() {
                    assert_eq!(
                        runtime
                            .memory
                            .read_u64(0x50000 + id as u64 * 256 + i as u64 * 8)
                            .unwrap(),
                        *expected as u64,
                        "thread={id} command={command}"
                    );
                }
                assert_eq!(observer.seals(*fd, None), Ok(15));
                assert_eq!(observer.pread(*fd, 7, 0).unwrap(), b"payload");
            }
            assert_eq!(observer.seals(busy, None), Ok(0));
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn original_posix_ofd_and_directory_locks_complete_without_joined_native_dispatch() {
        let imm = |code: &mut Vec<u32>, reg: u32, value: u64| {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | reg);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | reg);
                }
            }
        };
        // number, original FD register (31 = missing), command, lock kind, result.
        let calls = [
            (25, 22, 7, 1, 0i64),
            (25, 22, 5, 1, 0),
            (25, 22, 6, 2, 0),
            (25, 22, 38, 1, 0),
            (25, 22, 36, 1, 0),
            (25, 22, 37, 2, 0),
            (32, 25, 2, 0, 0),
            (32, 25, 8, 0, 0),
            (25, 26, 7, 1, -9),
            (25, 31, 38, 1, -9),
            (25, 25, 6, 1, -9),
            (32, 31, 0, 0, -22),
        ];
        for cpus in [4, 12] {
            let mut code = vec![0xd2a000b4, 0x8b132294]; // results: 0x50000+id*256
            imm(&mut code, 21, 0x51000);
            code.push(0x8b131ab5); // record: base+id*64
            imm(&mut code, 23, 0x52000);
            code.push(0xf8737af6); // original FD table
            for (i, (number, fd, command, kind, _)) in calls.iter().enumerate() {
                if *fd == 31 {
                    imm(&mut code, 0, u32::MAX as u64);
                } else {
                    code.push(0xaa0003e0 | (*fd << 16));
                }
                imm(&mut code, 1, *command);
                if *number == 25 {
                    code.push(0xaa1503e2);
                    imm(&mut code, 9, *kind);
                    code.push(0x790002a9);
                } else {
                    imm(&mut code, 2, 0);
                }
                imm(&mut code, 8, *number);
                code.extend([0xd4000001, 0xf9000280 | ((i as u32) << 10)]);
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime = Runtime::load(
                &threaded_image(cpus, &code),
                &["native-file-locks".into()],
                &[],
            )
            .unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 12288, trames_memory::Permissions::RW)
                .unwrap();
            let directory = runtime.linux.vfs.open_at(-100, b"/", 0, 0).unwrap();
            let path = runtime
                .linux
                .vfs
                .open_at(-100, b"/", trames_vfs::O_PATH, 0)
                .unwrap();
            use trames_execution::Registers;
            runtime.cpu.set_x(25, directory as u64);
            runtime.cpu.set_x(26, path as u64);
            let mut files = Vec::new();
            for id in 0..cpus {
                let fd = runtime
                    .linux
                    .vfs
                    .open_at(-100, b"/native-locks", 0x42, 0o600)
                    .unwrap();
                runtime
                    .memory
                    .write(0x52000 + id as u64 * 8, &(fd as u64).to_le_bytes())
                    .unwrap();
                let mut record = [0; 32];
                record[8..16].copy_from_slice(&(id as u64 * 16).to_le_bytes());
                record[16..24].copy_from_slice(&8u64.to_le_bytes());
                runtime
                    .memory
                    .write(0x51000 + id as u64 * 64, &record)
                    .unwrap();
                files.push(fd);
            }
            let observer = runtime.linux.vfs.clone_with_files(false);
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware file-lock CPUs={cpus} exit={} cohorts={} returned={:?}",
                exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(exit.code, 0);
            for (id, fd) in files.iter().enumerate() {
                for (i, (_, _, command, _, expected)) in calls.iter().enumerate() {
                    assert_eq!(
                        runtime
                            .memory
                            .read_u64(0x50000 + id as u64 * 256 + i as u64 * 8)
                            .unwrap(),
                        *expected as u64,
                        "thread={id} command={command}"
                    );
                }
                let requested = observer
                    .normalize_lock(*fd, 1, 0, id as i64 * 16, 8)
                    .unwrap();
                assert_eq!(observer.record_lock_conflict(*fd, requested), Ok(None));
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn guest_memfd_lifecycle_completes_on_original_native_workers() {
        let imm = |code: &mut Vec<u32>, register: u32, value: u64| {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | register);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | register);
                }
            }
        };
        type Call = (u64, [u64; 6], &'static [(u32, u32)], i64, Option<u32>);
        let calls: [Call; 20] = [
            (279, [0x54000, 3, 0, 0, 0, 0], &[], -1000, Some(23)),
            (46, [0, 1 << 30, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (64, [0, 0x54040, 7, 0, 0, 0], &[(0, 23)], 7, None),
            (25, [0, 1033, 6, 0, 0, 0], &[(0, 23)], 0, None),
            (
                222,
                [0, 4096, 3, 0x11, 0, 0],
                &[(0, 22), (4, 23)],
                -1001,
                None,
            ),
            (215, [0, 4096, 0, 0, 0, 0], &[(0, 22)], 0, None),
            (25, [0, 1033, 8, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1033, 1, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 15, None),
            (23, [0, 0, 0, 0, 0, 0], &[(0, 23)], -1000, Some(24)),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (279, [0x54010, 0, 0, 0, 0, 0], &[], -1000, Some(23)),
            (68, [0, 0x54040, 1, 0, 0, 0], &[(0, 24)], -1, None),
            (62, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (63, [0, 0, 7, 0, 0, 0], &[(0, 24), (1, 26)], 7, None),
            (64, [0, 0x54040, 1, 0, 0, 0], &[(0, 23)], 1, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 24)], 15, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 1, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
        ];
        for cpus in [4, 12] {
            let mut code = vec![0xd2a000b4, 0x8b132294]; // results = 0x50000 + id*256
            imm(&mut code, 22, 0x60000);
            code.push(0x8b1332d6); // mapping = base + id*4096
            imm(&mut code, 26, 0x52000);
            code.push(0x8b131f5a); // read buffer = base + id*128
            for (i, (number, args, registers, _, save)) in calls.iter().enumerate() {
                for (register, value) in args.iter().enumerate() {
                    imm(&mut code, register as u32, *value);
                }
                for (to, from) in *registers {
                    code.push(0xaa0003e0 | (*from << 16) | *to);
                }
                imm(&mut code, 8, *number);
                code.push(0xd4000001);
                if let Some(register) = save {
                    code.push(0xaa0003e0 | *register);
                }
                code.push(0xf9000280 | ((i as u32) << 10));
                if *number == 222 {
                    imm(&mut code, 9, 0x41424344);
                    code.push(0xb90002c9);
                }
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime =
                Runtime::load(&threaded_image(cpus, &code), &["native-memfd".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 5 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            for (at, bytes) in [
                (0x54000, b"guest-memory\0".as_slice()),
                (0x54010, b"replacement\0".as_slice()),
                (0x54040, b"payload".as_slice()),
            ] {
                runtime.memory.write(at, bytes).unwrap();
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware memfd CPUs={cpus} exit={} cohorts={} returned={:?} scopes={}",
                exit.code,
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_groups
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                for (i, (number, _, _, expected, _)) in calls.iter().enumerate() {
                    let value = runtime
                        .memory
                        .read_u64(0x50000 + id as u64 * 256 + i as u64 * 8)
                        .unwrap();
                    if *expected == -1000 {
                        assert!(
                            (3..4096).contains(&value),
                            "thread={id} call={i} fd={value}"
                        );
                    } else {
                        assert_eq!(
                            value,
                            if *expected == -1001 {
                                0x60000 + id as u64 * 4096
                            } else {
                                *expected as u64
                            },
                            "thread={id} syscall={number} call={i}"
                        );
                    }
                }
                assert_eq!(
                    runtime
                        .memory
                        .read_array::<7>(0x52000 + id as u64 * 128)
                        .unwrap(),
                    *b"DCBAoad"
                );
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn guest_msync_shared_backings_complete_on_original_native_workers() {
        let imm = |code: &mut Vec<u32>, register: u32, value: u64| {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | register);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | register);
                }
            }
        };
        type Call = (u64, [u64; 6], &'static [(u32, u32)], i64, Option<u32>);
        let calls: [Call; 24] = [
            (279, [0x54000, 3, 0, 0, 0, 0], &[], -1000, Some(23)),
            (46, [0, 1 << 30, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (64, [0, 0x54040, 7, 0, 0, 0], &[(0, 23)], 7, None),
            (25, [0, 1033, 6, 0, 0, 0], &[(0, 23)], 0, None),
            (
                222,
                [0, 4096, 3, 0x11, 0, 0],
                &[(0, 22), (4, 23)],
                -1001,
                None,
            ),
            (227, [0, 4096, 4, 0, 0, 0], &[(0, 22)], 0, None),
            (227, [0, 4096, 1, 0, 0, 0], &[(0, 22)], 0, None),
            (227, [0, 4096, 2, 0, 0, 0], &[(0, 22)], 0, None),
            (227, [0x1001, 0, 0, 0, 0, 0], &[], -22, None),
            (215, [0, 4096, 0, 0, 0, 0], &[(0, 22)], 0, None),
            (25, [0, 1033, 8, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1033, 1, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 15, None),
            (23, [0, 0, 0, 0, 0, 0], &[(0, 23)], -1000, Some(24)),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (279, [0x54010, 0, 0, 0, 0, 0], &[], -1000, Some(23)),
            (68, [0, 0x54040, 1, 0, 0, 0], &[(0, 24)], -1, None),
            (62, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (63, [0, 0, 7, 0, 0, 0], &[(0, 24), (1, 26)], 7, None),
            (64, [0, 0x54040, 1, 0, 0, 0], &[(0, 23)], 1, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 24)], 15, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 1, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
        ];
        for cpus in [4, 12] {
            let mut code = vec![0xd2a000b4, 0x8b132294]; // results = 0x50000 + id*256
            imm(&mut code, 22, 0x60000);
            code.push(0x8b1332d6); // mapping = base + id*4096
            imm(&mut code, 26, 0x52000);
            code.push(0x8b131f5a); // read buffer = base + id*128
            for (i, (number, args, registers, _, save)) in calls.iter().enumerate() {
                for (register, value) in args.iter().enumerate() {
                    imm(&mut code, register as u32, *value);
                }
                for (to, from) in *registers {
                    code.push(0xaa0003e0 | (*from << 16) | *to);
                }
                imm(&mut code, 8, *number);
                code.push(0xd4000001);
                if let Some(register) = save {
                    code.push(0xaa0003e0 | *register);
                }
                code.push(0xf9000280 | ((i as u32) << 10));
                if *number == 222 {
                    imm(&mut code, 9, 0x41424344);
                    code.push(0xb90002c9);
                }
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime =
                Runtime::load(&threaded_image(cpus, &code), &["native-msync".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 5 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            for (at, bytes) in [
                (0x54000, b"guest-memory\0".as_slice()),
                (0x54010, b"replacement\0".as_slice()),
                (0x54040, b"payload".as_slice()),
            ] {
                runtime.memory.write(at, bytes).unwrap();
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware msync CPUs={cpus} exit={} cohorts={} returned={:?} scopes={}",
                exit.code,
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_groups
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                for (i, (number, _, _, expected, _)) in calls.iter().enumerate() {
                    let value = runtime
                        .memory
                        .read_u64(0x50000 + id as u64 * 256 + i as u64 * 8)
                        .unwrap();
                    if *expected == -1000 {
                        assert!(
                            (3..4096).contains(&value),
                            "thread={id} call={i} fd={value}"
                        );
                    } else {
                        assert_eq!(
                            value,
                            if *expected == -1001 {
                                0x60000 + id as u64 * 4096
                            } else {
                                *expected as u64
                            },
                            "thread={id} syscall={number} call={i}"
                        );
                    }
                }
                assert_eq!(
                    runtime
                        .memory
                        .read_array::<7>(0x52000 + id as u64 * 128)
                        .unwrap(),
                    *b"DCBAoad"
                );
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn guest_file_cache_advice_completes_on_original_native_workers() {
        let imm = |code: &mut Vec<u32>, register: u32, value: u64| {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | register);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | register);
                }
            }
        };
        type Call = (u64, [u64; 6], &'static [(u32, u32)], i64, Option<u32>);
        let calls: [Call; 30] = [
            (279, [0x54000, 3, 0, 0, 0, 0], &[], -1000, Some(23)),
            (213, [0, 0, u64::MAX, 0, 0, 0], &[(0, 23)], 0, None),
            (213, [0, u64::MAX, 0, 0, 0, 0], &[(0, 23)], -22, None),
            (223, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 0, i64::MAX as u64, 1, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 1, 0, 2, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 1, 7, 3, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 1, 7, 4, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 0, 0, 5, 0, 0], &[(0, 23)], 0, None),
            (223, [0, 0, 0, 6, 0, 0], &[(0, 23)], -22, None),
            (223, [u32::MAX as u64, 0, 0, 6, 0, 0], &[], -9, None),
            (46, [0, 1 << 30, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (64, [0, 0x54040, 7, 0, 0, 0], &[(0, 23)], 7, None),
            (25, [0, 1033, 6, 0, 0, 0], &[(0, 23)], 0, None),
            (
                222,
                [0, 4096, 3, 0x11, 0, 0],
                &[(0, 22), (4, 23)],
                -1001,
                None,
            ),
            (215, [0, 4096, 0, 0, 0, 0], &[(0, 22)], 0, None),
            (25, [0, 1033, 8, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1033, 1, 0, 0, 0], &[(0, 23)], 0, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 15, None),
            (23, [0, 0, 0, 0, 0, 0], &[(0, 23)], -1000, Some(24)),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
            (279, [0x54010, 0, 0, 0, 0, 0], &[], -1000, Some(23)),
            (68, [0, 0x54040, 1, 0, 0, 0], &[(0, 24)], -1, None),
            (62, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (63, [0, 0, 7, 0, 0, 0], &[(0, 24), (1, 26)], 7, None),
            (64, [0, 0x54040, 1, 0, 0, 0], &[(0, 23)], 1, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 24)], 15, None),
            (25, [0, 1034, 0, 0, 0, 0], &[(0, 23)], 1, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 24)], 0, None),
            (57, [0, 0, 0, 0, 0, 0], &[(0, 23)], 0, None),
        ];
        for cpus in [4, 12] {
            let mut code = vec![0xd2a000b4, 0x8b132294]; // results = 0x50000 + id*256
            imm(&mut code, 22, 0x60000);
            code.push(0x8b1332d6); // mapping = base + id*4096
            imm(&mut code, 26, 0x52000);
            code.push(0x8b131f5a); // read buffer = base + id*128
            for (i, (number, args, registers, _, save)) in calls.iter().enumerate() {
                for (register, value) in args.iter().enumerate() {
                    imm(&mut code, register as u32, *value);
                }
                for (to, from) in *registers {
                    code.push(0xaa0003e0 | (*from << 16) | *to);
                }
                imm(&mut code, 8, *number);
                code.push(0xd4000001);
                if let Some(register) = save {
                    code.push(0xaa0003e0 | *register);
                }
                code.push(0xf9000280 | ((i as u32) << 10));
                if *number == 222 {
                    imm(&mut code, 9, 0x41424344);
                    code.push(0xb90002c9);
                }
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime = Runtime::load(
                &threaded_image(cpus, &code),
                &["native-file-advice".into()],
                &[],
            )
            .unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 5 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            for (at, bytes) in [
                (0x54000, b"guest-memory\0".as_slice()),
                (0x54010, b"replacement\0".as_slice()),
                (0x54040, b"payload".as_slice()),
            ] {
                runtime.memory.write(at, bytes).unwrap();
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware file-advice CPUs={cpus} exit={} cohorts={} returned={:?} scopes={}",
                exit.code,
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_groups
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                for (i, (number, _, _, expected, _)) in calls.iter().enumerate() {
                    let value = runtime
                        .memory
                        .read_u64(0x50000 + id as u64 * 256 + i as u64 * 8)
                        .unwrap();
                    if *expected == -1000 {
                        assert!(
                            (3..4096).contains(&value),
                            "thread={id} call={i} fd={value}"
                        );
                    } else {
                        assert_eq!(
                            value,
                            if *expected == -1001 {
                                0x60000 + id as u64 * 4096
                            } else {
                                *expected as u64
                            },
                            "thread={id} syscall={number} call={i}"
                        );
                    }
                }
                assert_eq!(
                    runtime
                        .memory
                        .read_array::<7>(0x52000 + id as u64 * 128)
                        .unwrap(),
                    *b"DCBAoad"
                );
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn guest_event_dispatch_completes_on_original_native_workers() {
        fn imm(code: &mut Vec<u32>, register: u32, value: u64) {
            code.push(0xd2800000 | ((value as u16 as u32) << 5) | register);
            for shift in 1..4 {
                let word = ((value >> (shift * 16)) & 0xffff) as u32;
                if word != 0 {
                    code.push(0xf2800000 | (shift << 21) | (word << 5) | register);
                }
            }
        }
        type Call = (u64, [u64; 6], &'static [(u32, u32, u16)], i64, Option<u32>);
        let calls: &[Call] = &[
            (20, [0; 6], &[], -1000, Some(22)),
            (20, [0; 6], &[], -1000, Some(23)),
            (19, [1, 0, 0, 0, 0, 0], &[], -1000, Some(24)),
            (
                21,
                [0, 1, 0, 0, 0, 0],
                &[(0, 23, 0), (2, 24, 0), (3, 21, 0)],
                0,
                None,
            ),
            (
                21,
                [0, 1, 0, 0, 0, 0],
                &[(0, 22, 0), (2, 23, 0), (3, 21, 0)],
                0,
                None,
            ),
            (
                22,
                [0, 0, 1, 0, 0, 0],
                &[(0, 22, 0), (1, 21, 0x70)],
                1,
                None,
            ),
            (
                73,
                [0, 1, 0, 0, 0, 0],
                &[(0, 21, 0x20), (2, 21, 0xa0)],
                1,
                None,
            ),
            (
                72,
                [0, 0, 0, 0, 0, 0],
                &[(0, 22, 1), (1, 21, 0x40), (4, 21, 0xa0)],
                1,
                None,
            ),
            (
                73,
                [0, 1, 0, 0, 0, 0],
                &[(0, 21, 0x90), (2, 21, 0xa0)],
                1,
                None,
            ),
            (21, [0, 2, 0, 0, 0, 0], &[(0, 24, 0), (2, 23, 0)], -22, None),
            (
                21,
                [0, 2, u32::MAX as u64, 0, 0, 0],
                &[(0, 22, 0)],
                -9,
                None,
            ),
            (
                21,
                [0, 1, 0, 0, 0, 0],
                &[(0, 23, 0), (2, 22, 0), (3, 21, 0)],
                -40,
                None,
            ),
            (21, [0, 2, 0, 0, 0, 0], &[(0, 22, 0), (2, 23, 0)], 0, None),
            (
                22,
                [0, 0, 1, 0, 0, 0],
                &[(0, 22, 0), (1, 21, 0x70)],
                0,
                None,
            ),
            (
                21,
                [0, 1, 0, 0, 0, 0],
                &[(0, 22, 0), (2, 23, 0), (3, 21, 0)],
                0,
                None,
            ),
            (
                286,
                [0, 0, 1, u64::MAX, 0, 0],
                &[(0, 24, 0), (1, 21, 0x50)],
                8,
                None,
            ),
            (
                22,
                [0, 0, 1, 0, 0, 0],
                &[(0, 22, 0), (1, 21, 0x70)],
                0,
                None,
            ),
            (
                287,
                [0, 0, 1, u64::MAX, 0, 2],
                &[(0, 24, 0), (1, 21, 0xb0)],
                -22,
                None,
            ),
            (
                22,
                [0, 0, 1, 0, 0, 0],
                &[(0, 22, 0), (1, 21, 0x70)],
                1,
                None,
            ),
            (57, [0; 6], &[(0, 24, 0)], 0, None),
            (57, [0; 6], &[(0, 23, 0)], 0, None),
            (57, [0; 6], &[(0, 22, 0)], 0, None),
        ];
        for cpus in [4, 12] {
            let mut code = Vec::new();
            imm(&mut code, 20, 0x50000);
            imm(&mut code, 21, 0x54000);
            code.push(0x8b000000 | (19 << 16) | (9 << 10) | (20 << 5) | 20);
            code.push(0x8b000000 | (19 << 16) | (8 << 10) | (21 << 5) | 21);
            for (i, (number, args, registers, _, save)) in calls.iter().enumerate() {
                for (register, value) in args.iter().enumerate() {
                    imm(&mut code, register as u32, *value);
                }
                for (to, from, offset) in *registers {
                    code.push(0x91000000 | (u32::from(*offset) << 10) | (*from << 5) | *to);
                }
                imm(&mut code, 8, *number);
                code.push(0xd4000001);
                if let Some(register) = save {
                    code.push(0xaa0003e0 | *register);
                }
                code.push(0xf9000280 | ((i as u32) << 10));
                if i == 2 {
                    code.push(0xb9000000 | (8 << 10) | (21 << 5) | 22); // pollfd.fd = root
                    imm(&mut code, 9, 1);
                    code.push(0x9ac02000 | (22 << 16) | (9 << 5) | 9); // fd_set
                    code.push(0xf9000000 | (8 << 10) | (21 << 5) | 9);
                }
            }
            code.extend([0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000]);
            let mut runtime = Runtime::load(
                &threaded_image(cpus, &code),
                &["native-event-dispatch".into()],
                &[],
            )
            .unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .memory
                .map(0x50000, 8 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            for id in 0..cpus {
                let base = 0x54000 + id as u64 * 256;
                for (offset, value) in [
                    (0, 1),
                    (8, 42),
                    (0x24, 1),
                    (0x50, base + 0x60),
                    (0x58, 8),
                    (0x68, 1),
                    (0x90, 999),
                    (0x94, 1),
                    (0xb0, base + 0x68),
                    (0xb8, 8),
                ] {
                    // 32-bit poll flags share a row with revents, so setting a
                    // u64 here also initializes its unused high half to zero.
                    runtime
                        .memory
                        .write_array(base + offset, value.to_le_bytes())
                        .unwrap();
                }
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            eprintln!(
                "hardware event-dispatch CPUs={cpus} exit={} cohorts={} returned={:?} scopes={}",
                exit.code,
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_groups
            );
            assert_eq!(exit.code, 0);
            for id in 0..cpus {
                for (i, (number, _, _, expected, _)) in calls.iter().enumerate() {
                    let value = runtime
                        .memory
                        .read_u64(0x50000 + id as u64 * 512 + i as u64 * 8)
                        .unwrap();
                    if *expected == -1000 {
                        assert!((3..64).contains(&value));
                    } else {
                        assert_eq!(
                            value, *expected as u64,
                            "thread={id} syscall={number} call={i}"
                        );
                    }
                }
                assert_eq!(
                    runtime.memory.read_u64(0x54060 + id as u64 * 256).unwrap(),
                    1
                );
                assert_eq!(
                    runtime.memory.read_u64(0x54078 + id as u64 * 256).unwrap(),
                    42
                );
            }
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }

    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn dormant_shared_futex_child_resumes_without_another_joined_native_cohort() {
        for cpus in [4, 12] {
            let image = threaded_image(
                1,
                &[
                    0xd2a00038, 0xd2800220, 0xd2800001, 0xd2800002, 0xd2800003, 0xd2800004,
                    0xd2801b88, 0xd4000001, 0xb40005a0, 0xb7f807e0, 0xaa1803e0, 0xd2800061,
                    0xd2800002, 0xd2800023, 0x91001304, 0xd2800005, 0xd2800c48, 0xd4000001,
                    0xf100041f, 0x54000060, 0xb4fffec0, 0x14000033, 0xaa1903e0, 0x91008301,
                    0xd2800102, 0xd28007e8, 0xd4000001, 0xf100201f, 0x54000581, 0xd2800220,
                    0xd2800001, 0xd2800002, 0xd2800003, 0xd2800004, 0xd2801b88, 0xd4000001,
                    0xb40003c0, 0xb7f80460, 0x91001300, 0xd2800021, 0xd2800022, 0xd2800c48,
                    0xd4000001, 0xf100041f, 0x54000381, 0xb9400b00, 0x34ffffe0, 0x7100041f,
                    0x54000301, 0xd2800000, 0xd2800bc8, 0xd4000001, 0x14000000, 0xaa1803e0,
                    0xd2800001, 0xd2800002, 0xd2800003, 0xd2800c48, 0xd4000001, 0xb50001a0,
                    0x52800020, 0xb9000b00, 0xd2800000, 0xd2800ba8, 0xd4000001, 0x14000000,
                    0x91003300, 0xd2801001, 0xd2800002, 0xd2800003, 0xd2800c48, 0xd4000001,
                    0xd2800c60, 0xd2800bc8, 0xd4000001, 0x14000000,
                ],
            );
            let mut runtime = Runtime::load(&image, &["dormant-shared-futex".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            runtime
                .linux
                .vfs
                .import_file(b"/dormant-futex", vec![0; 4096])
                .unwrap();
            let file = runtime
                .linux
                .vfs
                .open_at(-100, b"/dormant-futex", 2, 0)
                .unwrap();
            runtime
                .memory
                .map_backed(
                    0x10000,
                    runtime
                        .linux
                        .vfs
                        .mapping_pages(file, 0, 4096, true, true)
                        .unwrap(),
                    true,
                    trames_memory::Permissions::RW,
                    false,
                )
                .unwrap();
            // An OFD opened from legacy runtime-info has no source proc
            // payload provider. Its real read keeps the setup boundary explicit.
            runtime.linux.vfs.set_runtime_info(1, Default::default());
            // Keep an actual excluded provider for the cold setup; an ordinary
            // published global proc file now belongs to source execution.
            struct Unavailable;
            impl trames_vfs::ProcSource for Unavailable {
                fn available(&self, _: trames_vfs::ProcRefresh) -> bool {
                    false
                }
                fn observe(&self, _: trames_vfs::ProcQuery<'_>) -> trames_vfs::RuntimeInfo {
                    panic!("unavailable source provider")
                }
            }
            runtime
                .linux
                .vfs
                .set_proc_source(std::sync::Arc::new(Unavailable));
            let legacy = runtime
                .linux
                .vfs
                .open_at(-100, b"/proc/version", 0, 0)
                .unwrap();
            trames_execution::Registers::set_x(&mut runtime.cpu, 25, legacy as u64);
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            let marker = runtime.memory.read_array::<4>(0x10008).unwrap();
            eprintln!("hardware dormant CPUs={cpus} exit={} marker={marker:?} cohorts={} scopes={} returned={:?}",exit.code,runtime.metrics.parallel_rounds,runtime.metrics.vm_checkpoint_spaces,runtime.metrics.cohort_return_syscalls);
            assert_eq!(exit.code, 0);
            assert_eq!(marker, 1u32.to_le_bytes());
            assert_eq!(runtime.metrics.cohort_return_syscalls.get(&63), Some(&1));
            assert_eq!(
                runtime.metrics.parallel_rounds, 2,
                "one setup dispatch, then the original live queue"
            );
            assert_eq!(runtime.metrics.service_requests, 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn uring_signal_requests_keep_original_native_submitters_in_solo_and_live_cohorts() {
        for cpus in [1, 4, 12] {
            let image = threaded_image(
                cpus,
                &[
                    0xd2a00038, 0x8b132318, 0xd2800016, 0xd2800000, 0x91008301, 0xd2800002,
                    0xd2800103, 0xd28010e8, 0xd4000001, 0xaa0002d6, 0xf9400314, 0xaa1403e0,
                    0xd2800021, 0xd2800002, 0xd2800003, 0xd2800004, 0xd2800005, 0xd2803548,
                    0xd4000001, 0xd1000400, 0xaa0002d6, 0xd2801648, 0xd4000001, 0xaa0003f5,
                    0xaa1503e0, 0xd2800141, 0xd2801048, 0xd4000001, 0xaa0002d6, 0xaa1403e0,
                    0xd2800001, 0xd2800022, 0xd2800023, 0xd2800004, 0xd2800005, 0xd2803548,
                    0xd4000001, 0xaa0002d6, 0xf9400700, 0xb9400000, 0x51020000, 0xaa0002d6,
                    0xb9404300, 0x51002800, 0xaa0002d6, 0xb9404f00, 0x51000400, 0xaa0002d6,
                    0xf9006316, 0xd2800020, 0xf9006700, 0xaa1603e0, 0xd2800ba8, 0xd4000001,
                    0x14000000,
                ],
            );
            let mut runtime = Runtime::load(&image, &["uring-authority".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let file = trames_shared::SharedFile::anonymous().unwrap();
            file.set_len(16384).unwrap();
            let mapping = file.map(0, 16384).unwrap();
            // SAFETY: this retained initialized allocation owns the complete RW
            // scratch range; original SharedPage/HVF ownership admits its bytes.
            let resource = unsafe {
                trames_memory::SharedMapping::from_allocation(
                    mapping.allocation_pointer(),
                    mapping.allocation_len(),
                    mapping.clone(),
                )
            };
            runtime.memory.map_anonymous_live(0x10000, 4096).unwrap();
            runtime
                .memory
                .import_mapping(0x10000, 4096, resource)
                .unwrap();
            let signal = runtime.linux.vfs.signalfd(-1, 1 << 9, 0).unwrap();
            let mut rings = Vec::new();
            for cpu in 0..cpus {
                let (fd, ring) = runtime.linux.vfs.uring_create(4, 8).unwrap();
                let base = 0x20000 + cpu as u64 * 0x20000;
                for (address, offset, len) in [
                    (base, 0, ring.ring_bytes),
                    (base + 0x10000, 0x10000000, ring.sqe_bytes),
                ] {
                    runtime
                        .memory
                        .map_backed(
                            address,
                            runtime
                                .linux
                                .vfs
                                .mapping_pages(fd, offset, len, true, true)
                                .unwrap(),
                            true,
                            trames_memory::Permissions::RW,
                            false,
                        )
                        .unwrap();
                }
                let at = 0x10000 + cpu as u64 * 256;
                runtime
                    .memory
                    .write_array(at, (fd as u64).to_le_bytes())
                    .unwrap();
                runtime
                    .memory
                    .write_array(
                        at + 8,
                        (base + trames_vfs::URING_CQES as u64 + 8).to_le_bytes(),
                    )
                    .unwrap();
                runtime
                    .memory
                    .write_array(at + 32, (1u64 << 9).to_le_bytes())
                    .unwrap();
                let mut sqe = [0; 64];
                sqe[0] = 22;
                sqe[4..8].copy_from_slice(&signal.to_le_bytes());
                sqe[16..24].copy_from_slice(&(at + 64).to_le_bytes());
                sqe[24..28].copy_from_slice(&128u32.to_le_bytes());
                sqe[32..40].copy_from_slice(&(cpu as u64 + 101).to_le_bytes());
                runtime.memory.write(base + 0x10000, &sqe).unwrap();
                runtime
                    .memory
                    .write_array(base + 4, 1u32.to_le_bytes())
                    .unwrap();
                rings.push(ring);
            }
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            let completions: Vec<_> = rings
                .iter()
                .map(|ring| i32::from_le_bytes(ring.read_queue::<4>(trames_vfs::URING_CQES + 8)))
                .collect();
            eprintln!("hardware uring-authority CPUs={cpus} exit={} cq={completions:?} cohorts={} returned={:?}", exit.code, runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls);
            assert_eq!(exit.code, 0);
            assert_eq!(completions, vec![128; cpus]);
            for cpu in 0..cpus {
                // SAFETY: every guest actor returned at final group completion.
                // The aligned fields lie in this retained 16 KiB allocation.
                let error = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(cpu * 256 + 192)
                        .cast::<u64>()
                        .read()
                };
                let complete = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(cpu * 256 + 200)
                        .cast::<u64>()
                        .read()
                };
                assert_eq!(error, 0, "CPU={cpu}");
                assert_eq!(complete, 1, "CPU={cpu}");
            }
            assert_eq!(runtime.metrics.service_requests, 0);
            if cpus > 1 {
                assert_eq!(runtime.metrics.parallel_rounds, 1);
                assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            }
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn mqueue_paths_and_directories_run_without_joined_native_dispatch() {
        for cpus in [4, 12] {
            let image = threaded_image(
                cpus,
                &[
                    0xd2a00034, 0x8b132294, 0xd2a00055, 0xd2800016, 0x92800c60, 0xaa1503e1,
                    0xaa1403e2, 0xd2800003, 0xd28009e8, 0xd4000001, 0xaa0002d6, 0xaa1503e0,
                    0xaa1403e1, 0xd2800568, 0xd4000001, 0xaa0002d6, 0x92800c60, 0xaa1503e1,
                    0xd2880002, 0xd2800003, 0xd2800708, 0xd4000001, 0xaa0003f7, 0xaa1403e1,
                    0xd2802002, 0xd28007a8, 0xd4000001, 0xd100c000, 0xaa0002d6, 0xaa1703e0,
                    0xd2800001, 0xd2800002, 0xd28007c8, 0xd4000001, 0xaa0002d6, 0xaa1703e0,
                    0xd2800728, 0xd4000001, 0xaa0002d6, 0xf9007296, 0xd2800020, 0xf9007680,
                    0xaa1603e0, 0xd2800ba8, 0xd4000001, 0x14000000,
                ],
            );
            let mut runtime = Runtime::load(&image, &["mqueue".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let file = trames_shared::SharedFile::anonymous().unwrap();
            file.set_len(16384).unwrap();
            let mapping = file.map(0, 16384).unwrap();
            // SAFETY: this retained initialized allocation owns the complete RW
            // scratch range; the original SharedPage/HVF owner admits its bytes.
            let resource = unsafe {
                trames_memory::SharedMapping::from_allocation(
                    mapping.allocation_pointer(),
                    mapping.allocation_len(),
                    mapping.clone(),
                )
            };
            runtime.memory.map_anonymous_live(0x10000, 4096).unwrap();
            runtime
                .memory
                .import_mapping(0x10000, 4096, resource)
                .unwrap();
            runtime
                .memory
                .map(0x20000, 4096, trames_memory::Permissions::RW)
                .unwrap();
            runtime.memory.write(0x20000, b"/mq\0").unwrap();
            runtime.linux.vfs.mkdir_at(-100, b"/mq", 0o755).unwrap();
            runtime
                .linux
                .vfs
                .mount(None, b"/mq", Some(b"mqueue"), 14, None)
                .unwrap();
            runtime
                .memory
                .map(0x30000, 3 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            assert_eq!(exit.code, 0);
            for i in 0..cpus {
                // SAFETY: final group completion has returned every guest actor.
                // Both aligned fields lie inside the retained 16 KiB allocation.
                let error = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(i * 256 + 224)
                        .cast::<u64>()
                        .read()
                };
                let completed = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(i * 256 + 232)
                        .cast::<u64>()
                        .read()
                };
                assert_eq!(error, 0, "CPU={i}");
                assert_eq!(completed, 1, "CPU={i}");
            }
            eprintln!(
                "hardware mqueue CPUs={cpus} cohorts={} returned={:?} mapping={:?}",
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_calls
            );
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.vm_checkpoint_calls[5], 0);
            assert_eq!(runtime.metrics.vm_checkpoint_calls[7], 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn memory_queries_run_on_original_native_views_without_mapping_checkpoints() {
        for cpus in [4, 12] {
            let image = threaded_image(
                cpus,
                &[
                    0xd2a00034, 0x8b131e94, 0xaa1f03f6, 0xaa1403e0, 0xaa1f03e1, 0xaa1f03e2,
                    0xd2a00043, 0xd2800064, 0xd2801d88, 0xd4000001, 0xaa0002d6, 0xd2a00060,
                    0xd2860001, 0x91002282, 0xd2801d08, 0xd4000001, 0xaa0002d6, 0xd2a00040,
                    0xd2820001, 0xd2800062, 0xd2801d28, 0xd4000001, 0xaa0002d6, 0xf9003696,
                    0xd2800035, 0xf9003a95, 0xaa1603e0, 0xd2800ba8, 0xd4000001, 0x14000000,
                ],
            );
            let mut runtime = Runtime::load(&image, &["memory-query".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let file = trames_shared::SharedFile::anonymous().unwrap();
            file.set_len(16384).unwrap();
            let mapping = file.map(0, 16384).unwrap();
            // SAFETY: this retained initialized allocation owns the complete RW
            // scratch range; the original SharedPage/HVF owner admits its bytes.
            let resource = unsafe {
                trames_memory::SharedMapping::from_allocation(
                    mapping.allocation_pointer(),
                    mapping.allocation_len(),
                    mapping.clone(),
                )
            };
            runtime.memory.map_anonymous_live(0x10000, 4096).unwrap();
            runtime
                .memory
                .import_mapping(0x10000, 4096, resource)
                .unwrap();
            runtime.memory.reserve(0x20000, 4096, false).unwrap();
            runtime
                .memory
                .map(0x30000, 3 * 4096, trames_memory::Permissions::RW)
                .unwrap();
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            assert_eq!(exit.code, 0);
            for i in 0..cpus {
                // SAFETY: final group completion has returned every guest actor.
                // Both aligned fields lie inside the retained 16 KiB allocation.
                let error = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(i * 128 + 104)
                        .cast::<u64>()
                        .read()
                };
                let completed = unsafe {
                    mapping
                        .allocation_pointer()
                        .add(i * 128 + 112)
                        .cast::<u64>()
                        .read()
                };
                assert_eq!(error, 0, "CPU={i}");
                assert_eq!(completed, 1, "CPU={i}");
            }
            eprintln!(
                "hardware memory-query CPUs={cpus} cohorts={} returned={:?} mapping={:?}",
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls,
                runtime.metrics.vm_checkpoint_calls
            );
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            assert_eq!(runtime.metrics.vm_checkpoint_calls[5], 0);
            assert_eq!(runtime.metrics.vm_checkpoint_calls[7], 0);
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn guest_poweroff_retires_original_native_group_without_returned_syscall() {
        for cpus in [4, 12] {
            // Every CLONE_THREAD child runs native instructions. Only original
            // PID 1 (x19 == 0) requests guest halt. No host power API is called.
            let image = threaded_image(
                cpus,
                &[
                    0xb5000193,
                    0x52800000 | (0xdead << 5),
                    0x72a00000 | (0xfee1 << 5),
                    0x52800001 | (0x1969 << 5),
                    0x72a00001 | (0x2812 << 5),
                    0x52800002 | (0x0123 << 5),
                    0x72a00002 | (0xcdef << 5),
                    0xd28011c8,
                    0xd4000001,
                    0xd28009a0,
                    0xd2800bc8,
                    0xd4000001,
                    0x14000000,
                ],
            );
            let mut runtime = Runtime::load(&image, &["guest-poweroff".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let exit = runtime
                .run(&mut trames_linux::CapturedOutput::default(), u64::MAX)
                .unwrap();
            assert_eq!(
                exit.code, 0,
                "poweroff must not return to the following exit(77)"
            );
            eprintln!(
                "hardware poweroff CPUs={cpus} cohorts={} returned={:?}",
                runtime.metrics.parallel_rounds, runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn sysinfo_preserves_one_hardware_cohort_with_four_and_twelve_cpus() {
        struct Host {
            shared: trames_shared::Mapping,
            cpus: usize,
            started: std::time::Instant,
            done: bool,
            completed: u64,
        }
        impl trames_linux::GuestOutput for Host {
            fn source_console(&mut self) -> std::sync::Arc<dyn trames_vfs::ConsoleIo> {
                std::sync::Arc::new(trames_vfs::CapturedConsole::default())
            }
            fn write(&mut self, _: trames_linux::OutputFd, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len())
            }
            fn control_poll_interval(&self) -> std::time::Duration {
                std::time::Duration::from_millis(1)
            }
            fn poll_control(&mut self, _: &trames_vfs::Vfs) -> std::io::Result<bool> {
                // SAFETY: a retained 16 KiB shared allocation contains naturally
                // aligned AArch64 words. Each original guest actor writes its own
                // 128-byte slot; live volatile reads are approximate statistics.
                let counts: Vec<_> = (0..self.cpus)
                    .map(|i| unsafe {
                        self.shared
                            .allocation_pointer()
                            .add(i * 128 + 112)
                            .cast::<u64>()
                            .read_volatile()
                    })
                    .collect();
                self.completed = counts.iter().sum();
                assert!(
                    self.started.elapsed() < std::time::Duration::from_secs(5),
                    "hardware sysinfo failed to progress"
                );
                self.done = counts.iter().all(|&n| n > 0)
                    && self.started.elapsed() >= std::time::Duration::from_millis(200);
                Ok(self.done)
            }
            fn control(&mut self, _: &mut trames_vfs::Vfs) -> std::io::Result<bool> {
                Ok(!self.done)
            }
        }
        for cpus in [4, 12] {
            let image = threaded_image(
                cpus,
                &[
                    0xd2a00034, // mov x20, #0x10000
                    0x8b131e94, // add x20, x20, x19, lsl #7
                    0xaa1403e0, // mov x0, x20
                    0xd2801668, // mov x8, #179
                    0xd4000001, 0x910006b5, // add x21, x21, #1
                    0xf9003a95, // str x21, [x20, #112]
                    0x17fffffb,
                ],
            );
            let mut runtime = Runtime::load(&image, &["sysinfo".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let file = trames_shared::SharedFile::anonymous().unwrap();
            file.set_len(16384).unwrap();
            let mapping = file.map(0, 16384).unwrap();
            // SAFETY: retained initialized RW storage, admitted by the original
            // SharedPage/HVF mapping owner just like other native shared bytes.
            let resource = unsafe {
                trames_memory::SharedMapping::from_allocation(
                    mapping.allocation_pointer(),
                    mapping.allocation_len(),
                    mapping.clone(),
                )
            };
            runtime.memory.map_anonymous_live(0x10000, 4096).unwrap();
            runtime
                .memory
                .import_mapping(0x10000, 4096, resource)
                .unwrap();
            let mut host = Host {
                shared: mapping,
                cpus,
                started: std::time::Instant::now(),
                done: false,
                completed: 0,
            };
            assert!(matches!(
                runtime.run(&mut host, u64::MAX),
                Err(trames_session::RuntimeError::Cancelled)
            ));
            // All execution has returned before inspecting complete ABI records.
            for i in 0..cpus {
                // SAFETY: these fixed-width fields lie in the retained allocation;
                // cancellation above completed all original guest byte actors.
                let procs = unsafe {
                    host.shared
                        .allocation_pointer()
                        .add(i * 128 + 80)
                        .cast::<u16>()
                        .read()
                };
                assert_eq!(procs, cpus as u16);
            }
            eprintln!(
                "hardware sysinfo CPUs={cpus} completed={} cohorts={} returned={:?}",
                host.completed,
                runtime.metrics.parallel_rounds,
                runtime.metrics.cohort_return_syscalls
            );
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn host_pause_resume_preserves_one_hardware_cohort() {
        struct Pauses {
            counters: trames_shared::Mapping,
            previous: Vec<u64>,
            paused: bool,
            stable: usize,
            cycles: usize,
            since: Instant,
            deadline: Instant,
            done: bool,
        }
        impl GuestOutput for Pauses {
            fn source_console(&mut self) -> std::sync::Arc<dyn trames_vfs::ConsoleIo> {
                std::sync::Arc::new(trames_vfs::CapturedConsole::default())
            }
            fn paused(&self) -> bool {
                self.paused
            }
            fn control_poll_interval(&self) -> Duration {
                Duration::from_millis(2)
            }
            fn poll_control(&mut self, _: &trames_vfs::Vfs) -> io::Result<bool> {
                assert!(
                    Instant::now() < self.deadline,
                    "hardware pause/resume timed out: paused={} cycles={} stable={}",
                    self.paused,
                    self.cycles,
                    self.stable
                );
                let current: Vec<_> = (0..self.previous.len())
                    .map(|i| {
                        // SAFETY: this retained initialized mapping is page-aligned.
                        // Each guest thread writes one distinct aligned u64. Native
                        // ARM64 LDR/STR is atomic; volatile reads create no Rust byte
                        // references over concurrent external hardware stores.
                        unsafe {
                            std::ptr::read_volatile(
                                self.counters.pointer().add(i * 8).cast::<u64>(),
                            )
                        }
                    })
                    .collect();
                if self.paused {
                    if current == self.previous {
                        self.stable += 1;
                    } else {
                        assert!(self.stable < 3, "hardware guest executed while paused");
                        self.stable = 0;
                        self.previous = current;
                    }
                    if self.stable >= 3 && self.since.elapsed() >= Duration::from_millis(30) {
                        self.paused = false;
                        self.cycles += 1;
                        self.stable = 0;
                    }
                } else if current
                    .iter()
                    .zip(&self.previous)
                    .all(|(now, before)| now > before)
                {
                    if self.cycles == 3 {
                        self.done = true;
                        return Ok(true);
                    }
                    self.previous = current;
                    self.paused = true;
                    self.since = Instant::now();
                }
                Ok(false)
            }
            fn control(&mut self, _: &mut trames_vfs::Vfs) -> io::Result<bool> {
                Ok(!self.done)
            }
            fn write(&mut self, _: OutputFd, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
        }
        for cpus in [4, 12] {
            // x19 is unique for the parent and each CLONE_THREAD child.
            let image = threaded_image(
                cpus,
                &[
                    0xd2a00020, 0x8b130c00, 0xd2800001, 0x91000421, 0xf9000001, 0x17fffffe,
                ],
            );
            let mut runtime = Runtime::load(&image, &["host-pause".into()], &[]).unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let file = trames_shared::SharedFile::anonymous().unwrap();
            file.set_len(16384).unwrap();
            let mapping = file.map(0, 16384).unwrap();
            // SAFETY: the retained mapping owns a complete initialized RW page;
            // original SharedPage/HVF admission owns guest access and geometry.
            let resource = unsafe {
                trames_memory::SharedMapping::from_allocation(
                    mapping.allocation_pointer(),
                    mapping.allocation_len(),
                    mapping.clone(),
                )
            };
            runtime.memory.map_anonymous_live(0x10000, 4096).unwrap();
            runtime
                .memory
                .import_mapping(0x10000, 4096, resource)
                .unwrap();
            let mut host = Pauses {
                counters: mapping,
                previous: vec![0; cpus],
                paused: false,
                stable: 0,
                cycles: 0,
                since: Instant::now(),
                deadline: Instant::now() + Duration::from_secs(5),
                done: false,
            };
            assert!(matches!(
                runtime.run(&mut host, u64::MAX),
                Err(trames_session::RuntimeError::Cancelled)
            ));
            assert_eq!(host.cycles, 3);
            assert_eq!(runtime.metrics.parallel_rounds, 1);
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            eprintln!(
                "hardware pause CPUs={cpus} cycles={} cohorts={}",
                host.cycles, runtime.metrics.parallel_rounds
            );
        }
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn memory_pressure_preserves_one_live_cohort_with_four_and_twelve_cpus() {
        unsafe extern "C" {
            fn trames_hvf_inject_pressure_for_test(bits: u32);
        }
        // A native AArch64 ELF creates CLONE_THREAD peers, then all tasks keep
        // issuing getpid. No libc, network, external fixture or guest exit is used.
        struct Live {
            output: Output,
            polls: usize,
            injected: Option<trames_hvf::PagingStats>,
            observed: Option<trames_hvf::PagingStats>,
            deadline: Instant,
        }
        impl GuestOutput for Live {
            fn source_console(&mut self) -> std::sync::Arc<dyn trames_vfs::ConsoleIo> {
                self.output.source_console()
            }
            fn control_poll_interval(&self) -> Duration {
                Duration::from_millis(2)
            }
            fn poll_control(&mut self, vfs: &trames_vfs::Vfs) -> io::Result<bool> {
                assert!(
                    Instant::now() < self.deadline,
                    "pressure did not complete live"
                );
                self.polls += 1;
                if self.polls == 3 {
                    self.injected = Some(trames_hvf::paging_stats());
                    // Process-local notification latch; never system-wide pressure.
                    unsafe { trames_hvf_inject_pressure_for_test(4) };
                }
                assert!(
                    !self.output.poll_control(vfs)?,
                    "pressure requested a joined callback"
                );
                let now = trames_hvf::paging_stats();
                if self.polls >= 8
                    && self.output.pressure.is_none()
                    && self.injected.is_some_and(|old| {
                        now.page_out_bytes > old.page_out_bytes && now.page_ins > old.page_ins
                    })
                {
                    self.observed = Some(now);
                    return Ok(true); // Only the explicit test completion cancels.
                }
                Ok(false)
            }
            fn control(&mut self, vfs: &mut trames_vfs::Vfs) -> io::Result<bool> {
                if self.observed.is_some() {
                    Ok(false)
                } else {
                    self.output.control(vfs)
                }
            }
            fn write(&mut self, _: OutputFd, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
        }
        let _signals = super::super::lifecycle::Signals::install().unwrap();
        for cpus in [4, 12] {
            trames_hvf::take_memory_pressure().unwrap();
            let mut runtime = Runtime::load(
                &threaded_image(cpus, &[0xd2801588, 0xd4000001, 0x17fffffe]),
                &["pressure".into()],
                &[],
            )
            .unwrap();
            runtime.cpu_count = cpus;
            runtime.profile_syscalls = true;
            let mut output = Live {
                output: Output {
                    source: Some(std::sync::Arc::new(trames_vfs::CapturedConsole::default())),
                    exec: None,
                    init: None,
                    console: None,
                    terminals: None,
                    root_signal: None,
                    host_memory_total: None,
                    deadline: None,
                    window: None,
                    pressure: None,
                    next_stop_check: Instant::now(),
                    bridge: None,
                },
                polls: 0,
                injected: None,
                observed: None,
                deadline: Instant::now() + Duration::from_secs(5),
            };
            assert!(matches!(
                runtime.run(&mut output, u64::MAX),
                Err(trames_session::RuntimeError::Cancelled)
            ));
            assert!(runtime.metrics.peak_parallel_cpus >= 2);
            assert_eq!(
                runtime.metrics.parallel_rounds, 1,
                "pressure must not rebuild the cohort"
            );
            assert_eq!(runtime.metrics.service_requests, 0);
            assert!(runtime.metrics.cohort_return_syscalls.is_empty());
            let before = output.injected.unwrap();
            let after = output.observed.unwrap();
            eprintln!(
                "pressure cohort CPUs={cpus} peak={} rounds={} evicted={} page_ins={}",
                runtime.metrics.peak_parallel_cpus,
                runtime.metrics.parallel_rounds,
                after.page_out_bytes - before.page_out_bytes,
                after.page_ins - before.page_ins
            );
        }
    }
}
