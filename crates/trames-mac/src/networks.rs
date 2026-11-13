// SPDX-License-Identifier: GPL-3.0-or-later
//! Trames-owned private IP links. No host interfaces or packet service thread.
use super::environments::{self as env, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use trames_distro::environment as model;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bridge {
    version: u32,
    name: String,
    uuid: String,
    nat: bool,
    members: BTreeMap<String, u8>,
}
fn name(n: &str) -> Result<()> {
    if n.is_empty()
        || n.len() > 40
        || !n
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("invalid network name".into());
    }
    Ok(())
}
fn path(r: &Path, n: &str) -> Result<PathBuf> {
    name(n)?;
    Ok(r.join("networks").join(format!("{n}.toml")))
}
fn lock(r: &Path) -> Result<File> {
    env::private_dir(&r.join("networks"))?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(r.join("locks/networks"))?;
    f.lock()?;
    Ok(f)
}
fn load(r: &Path, n: &str) -> Result<Bridge> {
    let b: Bridge = model::decode(&fs::read_to_string(path(r, n)?)?)?;
    if b.version != 1 || b.name != n {
        return Err("invalid Trames network profile".into());
    }
    Ok(b)
}
fn transport(r: &Path, b: &Bridge) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}:{}", r.display(), b.uuid))
    );
    let p = PathBuf::from(format!(
        "/private/tmp/trames-net-{}-{}",
        unsafe { libc::geteuid() },
        &key[..24]
    ));
    env::private_dir(&p)?;
    let m = fs::symlink_metadata(&p)?;
    if m.uid() != unsafe { libc::geteuid() } || m.permissions().mode() & 0o077 != 0 {
        return Err("unsafe Trames network directory".into());
    }
    Ok(p)
}
pub fn arguments(r: &Path, c: &model::Manifest) -> Result<Vec<String>> {
    let n = c
        .network
        .bridge
        .as_deref()
        .ok_or("bridge network is missing")?;
    let _lock = lock(r)?;
    let mut b = load(r, n)?;
    let id = if let Some(id) = b.members.get(&c.environment.uuid) {
        *id
    } else {
        let id = (2..=254)
            .find(|id| !b.members.values().any(|v| v == id))
            .ok_or("bridge addresses exhausted")?;
        b.members.insert(c.environment.uuid.clone(), id);
        env::atomic_write(&path(r, n)?, model::encode(&b)?.as_bytes())?;
        id
    };
    let dir = transport(r, &b)?;
    // The environment's original lifetime lock is already held. Only an inode
    // left by its previous owner can exist at this stable address.
    match fs::remove_file(dir.join(format!("p{id}"))) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let mut args = vec![
        "--network-link".into(),
        dir.to_string_lossy().into_owned(),
        "--network-id".into(),
        id.to_string(),
        "--network-nat".into(),
        b.nat.to_string(),
    ];
    for p in &c.network.publish {
        args.extend([
            "--publish".into(),
            format!("{},{},{}", p.protocol, p.host, p.guest),
        ]);
    }
    Ok(args)
}
pub fn dispatch(args: &[String]) -> Result<u8> {
    let r = env::root()?;
    let _lock = lock(&r)?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["ls"] | ["ls", "--json"] => {
            let mut rows = vec![];
            for e in fs::read_dir(r.join("networks"))? {
                let p = e?.path();
                if p.extension().is_some_and(|s| s == "toml") {
                    let b: Bridge = model::decode(&fs::read_to_string(p)?)?;
                    rows.push(b);
                }
            }
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            if args.iter().any(|s| s == "--json") {
                println!("{}", serde_json::to_string_pretty(&rows)?)
            } else {
                println!("NAME                 NAT    MEMBERS");
                for b in rows {
                    println!("{:<20} {:<6} {}", b.name, b.nat, b.members.len());
                }
            }
        }
        ["create", n] | ["create", n, "--internal"] => {
            let p = path(&r, n)?;
            if p.exists() {
                return Err("network exists".into());
            }
            env::atomic_write(
                &p,
                model::encode(&Bridge {
                    version: 1,
                    name: (*n).into(),
                    uuid: model::uuid()?,
                    nat: args.len() == 2,
                    members: BTreeMap::new(),
                })?
                .as_bytes(),
            )?;
            println!("Created Trames network {n}");
        }
        ["inspect", n] => print!("{}", model::encode(&load(&r, n)?)?),
        ["remove", n] => {
            let b = load(&r, n)?;
            for id in b.members.keys() {
                if env::running(&r, id) {
                    return Err("stop attached environments before removing network".into());
                }
            }
            fs::remove_file(path(&r, n)?)?;
            let _ = fs::remove_dir_all(transport(&r, &b)?);
        }
        _ => return Err(
            "usage: network ls [--json] | create NAME [--internal] | inspect NAME | remove NAME"
                .into(),
        ),
    }
    Ok(0)
}
