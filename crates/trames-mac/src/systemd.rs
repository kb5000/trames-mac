// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional Debian container init policy. No hot guest I/O passes through here.
use serde::{Deserialize, Serialize};
use std::path::Path;
pub use trames_distro::systemd::{file, prepare, renew_identity, Monitor};
type Result<T> = super::environments::Result<T>;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Init {
    #[default]
    Shell,
    Systemd,
}
impl Init {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "shell" => Ok(Self::Shell),
            "systemd" => Ok(Self::Systemd),
            _ => Err("init must be shell or systemd".into()),
        }
    }
}

/// Provision once when opting in. Config is published only after success;
/// package-manager failure leaves a stopped environment available for repair.
pub fn install(store: &Path, cpus: usize, network: &str) -> Result<()> {
    eprintln!("Preparing original Debian systemd and D-Bus packages…");
    let code = super::execute(vec![
        "--storage".into(),
        store.to_string_lossy().into_owned(),
        "--cpus".into(),
        cpus.to_string(),
        "--network".into(),
        network.into(),
        "--no-windows".into(),
        "--no-pty".into(),
        "/bin/sh".into(),
        "-ec".into(),
        trames_distro::packages::debian_script("minimal", true)?,
    ])
    .map_err(|e| format!("systemd setup failed: {e}"))?;
    if code != 0 {
        return Err(format!("systemd package installation exited {code}; init setting was not changed. Retry with network access, or repair packages using --init shell.").into());
    }
    Ok(())
}
