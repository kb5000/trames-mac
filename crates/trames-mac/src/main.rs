// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal-first Mac composition of the shared Linux process kernel.
//! The Hypervisor adapter owns instruction execution and MMU transport.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod environments;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod files;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod host_memory;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod lifecycle;
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.first().is_some_and(|a| a == "env") {
        environments::dispatch(&args[1..]).map_err(|e| e.to_string())
    } else if args.first().is_some_and(|a| a == "--version") {
        println!(
            "Trames {} runtime {}",
            env!("CARGO_PKG_VERSION"),
            include_str!("../../../runtime-version.json")
        );
        Ok(0)
    } else if args.first().is_some_and(|a| a == "--manager")
        || (args.is_empty() && !std::io::stdin().is_terminal())
    {
        unsafe extern "C" {
            fn trames_manager_run();
        }
        unsafe {
            trames_manager_run();
        }
        Ok(0)
    } else if args.is_empty() {
        environments::dispatch(&["default".into()]).map_err(|e| e.to_string())
    } else {
        execute(args)
    };
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
            eprintln!("Use `trames-mac-run env doctor` for version and environment information.");
            std::process::exit(1);
        }
    }
}
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn execute(args: Vec<String>) -> Result<u8, String> {
    use std::io::IsTerminal;
    session::Command::parse(args.into_iter(), std::io::stdin().is_terminal())
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
