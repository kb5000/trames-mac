// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    println!("cargo:rerun-if-changed=src/window.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64")
    {
        return;
    }
    cc::Build::new()
        .file("src/window.m")
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .warnings(true)
        .extra_warnings(true)
        .compile("trames_mac_windows");
    for framework in [
        "AppKit",
        "Foundation",
        "CoreFoundation",
        "CoreGraphics",
        "Metal",
        "QuartzCore",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
