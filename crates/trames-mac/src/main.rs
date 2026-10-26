// SPDX-License-Identifier: GPL-3.0-or-later
//! Hardware backend bring-up CLI. Shares ELF/Linux/VFS/memory with the iPad;
//! process scheduling, signals and shared providers remain integration gates.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod apple;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    match apple::run() {
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
