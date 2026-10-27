// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal-first Mac composition of the shared Linux process kernel.
//! The Hypervisor adapter owns instruction execution and MMU transport.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod session;
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
                let bundled = directory.join("Trames.app/Contents/MacOS/trames-mac-run");
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
    use std::io::IsTerminal;
    let result = session::Command::parse(std::env::args().skip(1), std::io::stdin().is_terminal())
        .map_err(|e| e.to_string())
        .and_then(|command| match command {
            None => Ok(0),
            Some(command) if !command.windows => {
                // No presenter/run-loop source or separate session thread for servers.
                session::run(None, command).map_err(|e| e.to_string())
            }
            Some(command) => windows::run(command),
        });
    match result {
        Ok(code) => std::process::exit(code as i32),
        Err(error) => {
            eprintln!("trames-mac: {error}");
            std::process::exit(1);
        }
    }
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("trames-mac-run requires Apple Silicon macOS");
    std::process::exit(1);
}
