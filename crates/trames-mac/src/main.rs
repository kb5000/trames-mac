// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal-first Mac composition of the shared Linux process kernel.
//! The Hypervisor adapter owns instruction execution and MMU transport.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod background;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod cli;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod console;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod environments;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod exec;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod files;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod host_memory;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod lifecycle;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod packages;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod product;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod rootfs;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod session;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod systemd;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod windows;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    // Re-exec the paired bundle in the same process so AppKit has a stable
    // identity. No GUI is initialized until a Linux window is mapped.
    use std::os::unix::process::CommandExt;
    if std::env::var_os("TRAMES_MAC_BUNDLED").is_none() {
        if let Ok(executable) = std::env::current_exe() {
            if let Some(directory) = executable
                .parent()
                .filter(|p| p.file_name().is_some_and(|n| n != "MacOS"))
            {
                let bundled = directory
                    .join(product::bundle())
                    .join("Contents/MacOS/trames-mac-run");
                if bundled.is_file() {
                    let error = std::process::Command::new(bundled)
                        .args(std::env::args_os().skip(1))
                        .env("TRAMES_MAC_BUNDLED", "1")
                        .exec();
                    eprintln!("trames-mac: cannot enter application bundle: {error}");
                    std::process::exit(1);
                }
            }
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = cli::dispatch(args).map_err(|e| e.to_string());
    match result {
        Ok(code) => std::process::exit(code as i32),
        Err(error) => {
            eprintln!("trames-mac: {error}");
            if error.contains("virtual filesystem errno 28") {
                eprintln!("Storage is full: check host free space and the private store's logical capacity. Existing environments were not reset.");
            }
            if error.contains("virtual filesystem errno 16") {
                eprintln!("The private store is open in another Trames session. Close that session before managing the store.");
            }
            let usage = error.starts_with("usage:")
                || error.starts_with("unknown command")
                || error.starts_with("unknown exec option")
                || error.starts_with("unexpected argument");
            std::process::exit(if usage { 2 } else { 1 });
        }
    }
}
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn execute(args: Vec<String>) -> Result<u8, String> {
    execute_with_default(args, &["/bin/sh".into(), "-l".into()])
}
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn execute_with_default(args: Vec<String>, command: &[String]) -> Result<u8, String> {
    use std::io::IsTerminal;
    session::Command::parse_with_default(args.into_iter(), std::io::stdin().is_terminal(), command)
        .map_err(|e| e.to_string())
        .and_then(|c| match c {
            None => Ok(0),
            Some(c) if !c.windows => session::run(None, c).map_err(|e| e.to_string()),
            Some(c) => windows::run(c),
        })
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("trames-mac-run requires Apple Silicon macOS");
    std::process::exit(1);
}
