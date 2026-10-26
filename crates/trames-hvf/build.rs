// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    println!("cargo:rerun-if-changed=src/host.c");
    println!("cargo:rerun-if-changed=src/vectors.S");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64")
    {
        return;
    }
    cc::Build::new()
        .file("src/host.c")
        .file("src/vectors.S")
        .flag("-std=c11")
        .warnings(true)
        .extra_warnings(true)
        .compile("trames_hvf_host");
    println!("cargo:rustc-link-lib=framework=Hypervisor");
}
