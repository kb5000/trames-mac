// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit offline access to the existing VFS; never a background mirror.
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::Path,
};
use trames_vfs::{Vfs, AT_FDCWD, O_DIRECTORY};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn mkdirs(vfs: &mut Vfs, path: &[u8]) -> Result<()> {
    let mut current = Vec::new();
    for part in path.split(|&b| b == b'/').filter(|p| !p.is_empty()) {
        if part == b"." || part == b".." {
            return Err("mount target cannot contain dot components".into());
        }
        current.push(b'/');
        current.extend_from_slice(part);
        match vfs.stat_at(AT_FDCWD, &current) {
            Ok(stat) if kind(&stat) == 0o40000 => (),
            Ok(_) => return Err("mount target parent is not a directory".into()),
            Err(error) if error.0 == 2 => vfs.mkdir_at(AT_FDCWD, &current, 0o755)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn kind(stat: &[u8; 128]) -> u32 {
    u32::from_le_bytes(stat[16..20].try_into().unwrap()) & 0o170000
}
fn child(parent: &[u8], name: &[u8]) -> Vec<u8> {
    let mut result = parent.to_vec();
    if result.last() != Some(&b'/') {
        result.push(b'/');
    }
    result.extend_from_slice(name);
    result
}
fn names(vfs: &mut Vfs, path: &[u8]) -> Result<Vec<Vec<u8>>> {
    let fd = vfs.open_at(AT_FDCWD, path, O_DIRECTORY, 0)?;
    let result: Result<Vec<Vec<u8>>> = (|| {
        let mut result = Vec::new();
        loop {
            let bytes = vfs.getdents(fd, 64 * 1024)?;
            if bytes.is_empty() {
                break;
            }
            let mut offset = 0;
            while offset < bytes.len() {
                let size = u16::from_le_bytes(bytes[offset + 16..offset + 18].try_into().unwrap())
                    as usize;
                let name = bytes[offset + 19..offset + size]
                    .split(|&b| b == 0)
                    .next()
                    .unwrap();
                if name != b"." && name != b".." {
                    result.push(name.to_vec());
                }
                offset += size;
            }
        }
        Ok(result)
    })();
    vfs.close(fd)?;
    result
}
fn cat(vfs: &mut Vfs, path: &[u8], output: &mut impl Write) -> Result<()> {
    let fd = vfs.open_at(AT_FDCWD, path, 0, 0)?;
    let result: Result<()> = (|| {
        loop {
            let bytes = vfs.read(fd, 64 * 1024)?;
            if bytes.is_empty() {
                break;
            }
            output.write_all(&bytes)?;
        }
        output.flush()?;
        Ok(())
    })();
    vfs.close(fd)?;
    result
}
fn get(vfs: &mut Vfs, guest: &[u8], host: &Path) -> Result<()> {
    let stat = vfs.stat_at_flags(AT_FDCWD, guest, true)?;
    match kind(&stat) {
        0o40000 => {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(host)?;
            for name in names(vfs, guest)? {
                get(
                    vfs,
                    &child(guest, &name),
                    &host.join(std::ffi::OsString::from_vec(name)),
                )?;
            }
        }
        0o100000 => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(host)?;
            cat(vfs, guest, &mut file)?;
            file.sync_all()?;
        }
        0o120000 => std::os::unix::fs::symlink(
            std::ffi::OsString::from_vec(vfs.readlink_at(AT_FDCWD, guest)?),
            host,
        )?,
        _ => return Err("only regular files, directories and symlinks can be copied".into()),
    }
    Ok(())
}
fn put(vfs: &mut Vfs, host: &Path, guest: &[u8], replace: bool) -> Result<()> {
    let meta = fs::symlink_metadata(host)?;
    if meta.is_dir() {
        if replace {
            return Err("--replace accepts a regular file only".into());
        }
        vfs.mkdir_at(AT_FDCWD, guest, 0o755)?;
        for entry in fs::read_dir(host)? {
            let entry = entry?;
            put(
                vfs,
                &entry.path(),
                &child(guest, entry.file_name().as_bytes()),
                false,
            )?;
        }
    } else if meta.file_type().is_symlink() {
        if replace {
            return Err("--replace accepts a regular file only".into());
        }
        vfs.symlink_at(fs::read_link(host)?.as_os_str().as_bytes(), AT_FDCWD, guest)?;
    } else if meta.is_file() {
        if guest.ends_with(b"/") {
            return Err("destination file cannot end in '/'".into());
        }
        let parent = &guest[..guest
            .iter()
            .rposition(|&b| b == b'/')
            .ok_or("use an absolute guest destination")?
            .max(1)];
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temporary = child(
            parent,
            format!(".trames-upload-{}-{stamp}-{nonce}", std::process::id()).as_bytes(),
        );
        let mut source = fs::File::open(host)?;
        let fd = vfs.open_at(AT_FDCWD, &temporary, 0xc1, 0o644)?;
        let result: Result<()> = (|| {
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                let mut position = 0;
                while position < count {
                    let written = vfs.write(fd, &buffer[position..count])?;
                    if written == 0 {
                        return Err("zero-length guest write".into());
                    }
                    position += written;
                }
            }
            vfs.sync_fd(fd, false)?;
            vfs.rename_at(AT_FDCWD, &temporary, AT_FDCWD, guest, u32::from(!replace))?;
            Ok(())
        })();
        vfs.close(fd)?;
        if result.is_err() {
            let _ = vfs.unlink_at(AT_FDCWD, &temporary, false);
        }
        result?;
    } else {
        return Err("only regular files, directories and symlinks can be copied".into());
    }
    Ok(())
}
pub fn run(vfs: &mut Vfs, args: &[String]) -> Result<()> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["ls"] => {
            for name in names(vfs, b"/")? {
                io::stdout().write_all(&name)?;
                println!();
            }
        }
        ["ls", guest] => {
            for name in names(vfs, guest.as_bytes())? {
                io::stdout().write_all(&name)?;
                println!();
            }
        }
        ["cat", guest] => cat(vfs, guest.as_bytes(), &mut io::stdout().lock())?,
        ["get", guest, host] => get(vfs, guest.as_bytes(), Path::new(host))?,
        ["put", host, guest] => {
            put(vfs, Path::new(host), guest.as_bytes(), false)?;
            vfs.sync()?;
        }
        ["put", "--replace", host, guest] => {
            put(vfs, Path::new(host), guest.as_bytes(), true)?;
            vfs.sync()?;
        }
        _ => {
            return Err(
                "usage: --fs ls [GUEST] | cat GUEST | get GUEST HOST | put [--replace] HOST GUEST"
                    .into(),
            )
        }
    }
    Ok(())
}
