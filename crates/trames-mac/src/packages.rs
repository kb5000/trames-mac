// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit, cold Debian package setup; never run on environment startup.
use super::environments::Result;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Minimal,
    Standard,
    Development,
}
impl Profile {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "minimal" => Ok(Self::Minimal),
            "standard" => Ok(Self::Standard),
            "development" => Ok(Self::Development),
            _ => Err("packages must be minimal, standard or development".into()),
        }
    }
}
pub fn install(store: &Path, cpus: usize, network: &str, profile: Profile) -> Result<()> {
    if profile == Profile::Minimal {
        return Ok(());
    }
    eprintln!(
        "Preparing Debian {} tools…",
        if profile == Profile::Development {
            "development"
        } else {
            "daily"
        }
    );
    let script = trames_distro::packages::debian_script(
        if profile == Profile::Development {
            "development"
        } else {
            "standard"
        },
        false,
    )?;
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
        script,
    ])
    .map_err(|e| format!("Debian tool setup failed: {e}"))?;
    if code != 0 {
        return Err(format!("Debian tool installation exited {code}. Retry with network access using env packages NAME; use --packages minimal for an offline base image. Existing guest files are preserved.").into());
    }
    Ok(())
}
