// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};
use trames_linux::{GuestOutput, OutputFd, TerminalEvent};
use trames_session::{MemoryLimits, RuntimeOptions, Terminal};
type Runtime = trames_session::Runtime<trames_hvf::kernel::Cpu>;

struct Output {
    deadline: Instant,
    window: Option<(u16, u16)>,
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
    fn poll_control(&mut self, _: &trames_vfs::Vfs) -> io::Result<bool> {
        Ok(Instant::now() >= self.deadline)
    }
    fn control(&mut self, _: &mut trames_vfs::Vfs) -> io::Result<bool> {
        Ok(Instant::now() < self.deadline)
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

pub fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1).peekable();
    let mut rootfs = None;
    let mut storage = None;
    let mut imports = Vec::new();
    let mut network = trames_vfs::NetworkMode::Full;
    let mut trace = false;
    let mut metrics = false;
    let mut timeout = 86400;
    let mut limit = 4usize << 30;
    let mut pty = io::stdin().is_terminal();
    while arguments.peek().is_some_and(|a| a.starts_with("--")) {
        match arguments.next().unwrap().as_str() {
            "--rootfs" => rootfs = Some(arguments.next().ok_or("missing --rootfs archive")?),
            "--storage" => storage = Some(arguments.next().ok_or("missing storage directory")?),
            "--exec-file" => imports.push(arguments.next().ok_or("missing GUEST=HOST import")?),
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
            "--memory-mib" => {
                limit = arguments
                    .next()
                    .ok_or("missing memory limit")?
                    .parse::<usize>()?
                    .checked_mul(1 << 20)
                    .ok_or("memory limit overflow")?
            }
            "--trace" => trace = true,
            "--metrics" => metrics = true,
            "--pty" => pty = true,
            "--no-pty" => pty = false,
            "--" => break,
            "--help" => {
                println!("trames-mac-run [--rootfs Alpine.tar.gz] [--storage DIR] [--exec-file GUEST=HOST] [--network off|loopback|outbound|full] [--memory-mib N] [--timeout SECONDS] [--pty|--no-pty] [--trace] [--metrics] [ELF [ARG...]]\nLinux userspace with hardware ARM64 execution; defaults to /bin/sh -l with a rootfs.");
                return Ok(0);
            }
            _ => return Err("unknown option".into()),
        }
    }
    let mut argv: Vec<String> = arguments.collect();
    if argv.is_empty() && (rootfs.is_some() || storage.is_some()) {
        argv = vec!["/bin/sh".into(), "-l".into()];
    }
    let path = argv
        .first()
        .ok_or("an ELF program or --rootfs is required; use --help")?;
    let options = RuntimeOptions {
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
    vfs.configure_network(network)?;
    if let Some(archive) = &rootfs {
        if vfs
            .stat_at(trames_vfs::AT_FDCWD, b"/etc/alpine-release")
            .is_err()
        {
            trames_distro::install_alpine(&mut std::fs::File::open(archive)?, &mut vfs)?;
        }
    }
    for import in imports {
        let (guest, host) = import.split_once('=').ok_or("import must be GUEST=HOST")?;
        vfs.import_reader(guest.as_bytes(), &mut std::fs::File::open(host)?, true)?;
    }
    let bytes = if rootfs.is_some() || storage.is_some() {
        vfs.executable(path.as_bytes())?
    } else {
        let bytes = std::fs::read(path)?;
        vfs.import_executable(b"/program", bytes.clone())?;
        bytes
    };
    let envp = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        "HOME=/root".into(),
        "LANG=C.UTF-8".into(),
        "TERM=xterm-256color".into(),
    ];
    let mut runtime = Runtime::load_with_options(&bytes, &argv, &envp, vfs, options)?;
    runtime.linux.trace = trace;
    runtime.cpu_count = 1;
    runtime.profile_syscalls = metrics;
    if pty {
        runtime.enable_terminal(Terminal::default())?;
    }
    let _terminal = RawTerminal::enter(pty)?;
    let mut output = Output {
        deadline: Instant::now() + Duration::from_secs(timeout),
        window: None,
    };
    let result = runtime.run(&mut output, u64::MAX);
    if metrics {
        eprintln!("trames mac metrics: {{\"syscalls\":{},\"context_switches\":{},\"hardware_mapped_bytes\":{},\"guest_resident_bytes\":{},\"instruction_count\":null}}", runtime.metrics.syscalls, runtime.metrics.context_switches, runtime.engine.mapped_bytes(), runtime.memory.resident_bytes());
    }
    runtime.linux.vfs.sync()?;
    result.map(|exit| exit.code).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
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
