// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal-first Mac composition of the shared Linux process kernel.
//! The Hypervisor adapter owns instruction execution and MMU transport.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod session;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    match session::run() {
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
