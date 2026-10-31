// SPDX-License-Identifier: GPL-3.0-or-later
//! Build-time product identity; normal execution adds no mutable channel state.
use std::path::{Path, PathBuf};
pub const CHANNEL: &str = env!("TRAMES_BUILD_CHANNEL");
pub fn name() -> &'static str {
    if CHANNEL == "stable" {
        "Trames"
    } else {
        "Trames Dev"
    }
}
pub fn bundle() -> &'static str {
    if CHANNEL == "stable" {
        "Trames.app"
    } else {
        "Trames Dev.app"
    }
}
pub fn registry(home: &Path) -> PathBuf {
    home.join("Library/Application Support").join(name())
}
#[cfg(test)]
mod tests {
    #[test]
    fn channel_identity_is_immutable_and_uses_its_own_registry() {
        let path = super::registry(std::path::Path::new("/test-user"));
        assert_eq!(path.file_name().unwrap(), super::name());
        assert_eq!(super::bundle(), format!("{}.app", super::name()));
        assert!(matches!(super::CHANNEL, "stable" | "dev"));
    }
}
