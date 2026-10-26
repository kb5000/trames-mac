// SPDX-License-Identifier: GPL-3.0-or-later
//! Apple Silicon instruction execution. Linux and virtual-memory policy remain
//! in the common runtime; this adapter owns only vCPU and MMU transport.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub use macos::*;

pub const SUPPORTED: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub mod kernel;
