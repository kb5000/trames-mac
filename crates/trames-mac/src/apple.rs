// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};
use trames_execution::{Aarch64State, ExitReason};
use trames_linux::{GuestOutput, OutputFd, SyscallOutcome};
struct Output;
impl GuestOutput for Output {
    fn write(&mut self, fd: OutputFd, bytes: &[u8]) -> io::Result<usize> {
        match fd {
            OutputFd::Stdout => io::stdout().write(bytes),
            OutputFd::Stderr => io::stderr().write(bytes),
        }
    }
    fn read_stdin(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        io::stdin().read(bytes)
    }
}
pub fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1).peekable();
    let mut rootfs = None;
    let mut trace = false;
    let mut metrics = false;
    let mut timeout = 60;
    let mut limit = 4usize << 30;
    while arguments.peek().is_some_and(|a| a.starts_with("--")) {
        match arguments.next().unwrap().as_str() {
            "--rootfs" => rootfs = Some(arguments.next().ok_or("missing --rootfs archive")?),
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
            "--" => break,
            "--help" => {
                println!("trames-mac-run [--rootfs Alpine.tar.gz] [--memory-mib N] [--timeout SECONDS] [--trace] [--metrics] ELF [ARG...]\nSingle-process hardware bring-up; uses the common Linux ABI, not a booted Linux kernel.");
                return Ok(0);
            }
            _ => return Err("unknown option".into()),
        }
    }
    let argv: Vec<Vec<u8>> = arguments.map(String::into_bytes).collect();
    let path = argv
        .first()
        .ok_or("an ELF program is required; use --help")?;
    let mut vfs = trames_vfs::Vfs::default();
    if let Some(archive) = &rootfs {
        trames_distro::install_alpine(&mut std::fs::File::open(archive)?, &mut vfs)?;
    }
    let bytes = if rootfs.is_some() {
        vfs.executable(path)?
    } else {
        let bytes = std::fs::read(std::str::from_utf8(path)?)?;
        vfs.import_executable(b"/program", bytes.clone())?;
        bytes
    };
    let mut elf = trames_elf::load_with_interpreter(&bytes, limit, |name| {
        vfs.executable(name)
            .map_err(|e| trames_elf::ElfError::Interpreter(e.0))
    })?;
    let mut executor = trames_hvf::Executor::new()?;
    let envp = vec![
        b"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_vec(),
        b"HOME=/root".to_vec(),
        b"LANG=C.UTF-8".to_vec(),
    ];
    let sp = trames_linux::image::build_initial_stack(
        &mut elf,
        &argv,
        &envp,
        &vfs.credentials,
        executor.features(),
        &[],
        None,
    )
    .map_err(|e| format!("Linux process image: {e:?}"))?;
    let mut state = Aarch64State {
        pc: elf.start_entry,
        sp,
        ..Default::default()
    };
    let mut linux = trames_linux::Linux::new(elf.brk_base);
    linux.vfs = vfs;
    linux.arguments = argv.clone();
    linux.environment = envp;
    linux.executable_path = path.clone();
    linux.command_name = path
        .rsplit(|b| *b == b'/')
        .next()
        .unwrap_or_default()
        .to_vec();
    linux.trace = trace;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut syscalls = 0u64;
    let mut yields = 0u64;
    let mut output = Output;
    loop {
        if Instant::now() >= deadline {
            return Err("execution timeout".into());
        }
        match executor
            .run(&mut state, &mut elf.memory, Duration::from_millis(2))?
            .reason
        {
            ExitReason::Yield => yields += 1,
            ExitReason::Syscall => {
                syscalls += 1;
                match linux.dispatch(&mut state, &mut elf.memory, &mut output) {
                    SyscallOutcome::Continue => {}
                    SyscallOutcome::Exit(code) => {
                        if metrics {
                            eprintln!("trames mac metrics: {{\"syscalls\":{syscalls},\"yields\":{yields},\"hardware_mapped_bytes\":{},\"guest_resident_bytes\":{},\"instruction_count\":null}}",executor.mapped_bytes(),elf.memory.resident_bytes());
                        }
                        return Ok(code);
                    }
                }
            }
        }
    }
}
