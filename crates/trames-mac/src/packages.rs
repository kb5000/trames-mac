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
const STANDARD: &str = "ca-certificates curl wget git openssh-client nano vim-tiny less procps psmisc iproute2 iputils-ping bind9-dnsutils netcat-openbsd rsync unzip zip xz-utils file bash-completion tzdata";
const DEVELOPMENT: &str =
    "build-essential pkgconf python3 python3-venv python3-pip cmake ninja-build";

pub fn install(store: &Path, cpus: usize, network: &str, profile: Profile) -> Result<()> {
    if profile == Profile::Minimal {
        return Ok(());
    }
    let packages = if profile == Profile::Development {
        format!("{STANDARD} {DEVELOPMENT}")
    } else {
        STANDARD.into()
    };
    eprintln!(
        "Preparing Debian {} tools…",
        if profile == Profile::Development {
            "development"
        } else {
            "daily"
        }
    );
    // Constant package names only. Check the guest package database, not a host
    // marker: manual removals or interrupted apt runs can be repaired by retry.
    let script = format!(
        r#"ready=yes
for package in {packages}; do
    if [ "$(dpkg-query -W -f='${{Status}}' "$package" 2>/dev/null)" != 'install ok installed' ]; then ready=no; break; fi
done
if [ "$ready" = yes ]; then echo 'Debian tools already installed.'; exit 0; fi
export DEBIAN_FRONTEND=noninteractive
apt-get -o Acquire::Retries=1 -o Acquire::http::Timeout=30 update --error-on=any
apt-get install -y --no-install-recommends {packages}
apt-get clean
"#
    );
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
