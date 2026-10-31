// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional Debian container init policy. No hot guest I/O passes through here.
use serde::{Deserialize, Serialize};
use std::{path::Path, rc::Rc};
use trames_vfs::{LocalSocket, Vfs, AT_FDCWD};
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
        "--storage".into(), store.to_string_lossy().into_owned(),
        "--cpus".into(), cpus.to_string(), "--network".into(), network.into(),
        "--no-windows".into(), "--no-pty".into(), "/bin/sh".into(), "-ec".into(),
        "if ! dpkg-query -W -f='${Status}\\n' systemd dbus 2>/dev/null | grep -qv '^install ok installed$' && test -x /usr/lib/systemd/systemd && test -x /usr/bin/dbus-daemon; then exit 0; fi; export DEBIAN_FRONTEND=noninteractive; apt-get update; apt-get install -y --no-install-recommends systemd dbus".into(),
    ]).map_err(|e| format!("systemd setup failed: {e}"))?;
    if code != 0 {
        return Err(format!("systemd package installation exited {code}; init setting was not changed. Retry with network access, or repair packages using --init shell.").into());
    }
    Ok(())
}
/// A new registry entry is a clone, even when restored from a backup. Give it
/// an independent machine identity without touching the source environment.
pub fn renew_identity(vfs: &mut Vfs) -> Result<()> {
    use std::io::Read;
    if vfs.stat_at(AT_FDCWD, b"/etc/machine-id").is_err() {
        return Ok(());
    }
    let mut random = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let identity = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        + "\n";
    for path in [b"/etc/machine-id".as_slice(), b"/var/lib/dbus/machine-id"] {
        if vfs.stat_at(AT_FDCWD, path).is_ok() {
            let fd = vfs.open_at(AT_FDCWD, path, 0x201, 0)?;
            vfs.write(fd, identity.as_bytes())?;
            vfs.close(fd)?;
        }
    }
    vfs.sync()?;
    Ok(())
}

fn directory(vfs: &mut Vfs, path: &[u8], mode: u32) -> Result<()> {
    vfs.mkdir_at(AT_FDCWD, path, mode)?;
    Ok(())
}
pub fn file(vfs: &mut Vfs, path: &[u8], bytes: &[u8]) -> Result<()> {
    let fd = vfs.open_at(AT_FDCWD, path, 0xc1, 0o644)?;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let n = vfs.write(fd, remaining)?;
        remaining = &remaining[n..];
    }
    vfs.close(fd)?;
    Ok(())
}
fn bind(vfs: &mut Vfs, source: &[u8], destination: &[u8]) -> Result<()> {
    let fd = vfs.open_at(AT_FDCWD, destination, 0x42, 0o600)?;
    vfs.close(fd)?;
    vfs.mount(Some(source), destination, None, 4096, None)?;
    Ok(())
}

/// Match systemd's container interface using only guest VFS objects. /run and
/// /dev are per boot; no host mount, device or shutdown operation is involved.
pub fn prepare(vfs: &mut Vfs) -> Result<Monitor> {
    if vfs.stat_at(AT_FDCWD, b"/etc/debian_version").is_err()
        || vfs.executable(b"/usr/lib/systemd/systemd").is_err()
    {
        return Err(
            "systemd requires a prepared Debian environment; use env configure NAME --init systemd"
                .into(),
        );
    }
    // Seed pseudo mounts before loading PID 1; the scheduler publishes live
    // process and memory metadata on entry.
    vfs.set_runtime_info(1, Default::default());
    vfs.enable_terminal()?;
    vfs.mount(
        Some(b"tmpfs"),
        b"/run",
        Some(b"tmpfs"),
        2 | 4,
        Some(b"mode=755,size=0,nr_inodes=0"),
    )?;
    bind(vfs, b"/dev/pts/0", b"/run/console")?;
    vfs.mount(
        Some(b"tmpfs"),
        b"/dev",
        Some(b"tmpfs"),
        2,
        Some(b"mode=755,size=0,nr_inodes=0"),
    )?;
    let umask = vfs.set_umask(0);
    for (name, major, minor) in [
        ("null", 1, 3),
        ("zero", 1, 5),
        ("full", 1, 7),
        ("random", 1, 8),
        ("urandom", 1, 9),
        ("tty", 5, 0),
    ] {
        vfs.mknod_at(
            AT_FDCWD,
            format!("/dev/{name}").as_bytes(),
            0o20666,
            major * 256 + minor,
        )?;
    }
    vfs.set_umask(umask);
    bind(vfs, b"/run/console", b"/dev/console")?;
    directory(vfs, b"/dev/pts", 0o755)?;
    directory(vfs, b"/dev/shm", 0o1777)?;
    vfs.mount(
        Some(b"devpts"),
        b"/dev/pts",
        Some(b"devpts"),
        2 | 8,
        Some(b"newinstance,mode=620,ptmxmode=666"),
    )?;
    vfs.symlink_at(b"pts/ptmx", AT_FDCWD, b"/dev/ptmx")?;
    vfs.symlink_at(b"/proc/self/fd", AT_FDCWD, b"/dev/fd")?;
    vfs.mount(
        Some(b"tmpfs"),
        b"/dev/shm",
        Some(b"tmpfs"),
        2 | 4,
        Some(b"mode=1777,size=0,nr_inodes=0"),
    )?;
    vfs.mount(None, b"/sys", None, 32 | 1 | 2 | 4 | 8, None)?;
    // No module loader exists in this userspace container. Stock units observe
    // the real capability state, rather than requiring masks or edited units.
    let module = 1u64 << 16; // Linux CAP_SYS_MODULE
    vfs.credentials.caps.bounding &= !module;
    vfs.credentials.caps.permitted &= !module;
    vfs.credentials.caps.effective &= !module;
    for path in [
        b"/run/systemd".as_slice(),
        b"/run/systemd/system",
        b"/run/systemd/system/console-getty.service.d",
        b"/run/systemd/system/systemd-logind.service.d",
        b"/run/host",
    ] {
        directory(vfs, path, 0o755)?;
    }
    // Like shell mode, the local owner gets a root console without a guest
    // password. Keep the original service's TTY, lifetime and restart policy.
    file(vfs, b"/run/systemd/system/console-getty.service.d/50-trames.conf", b"[Service]\nExecStart=\nExecStart=/bin/sh -l\nEnvironment=HOME=/root TERM=xterm-256color LANG=C.UTF-8\n")?;
    // macOS/Trames owns consoles and devices. Linux seat/session management
    // requires shared mount propagation, which the common VFS does not yet
    // implement. Declare this container policy explicitly; retain other units'
    // namespace, capability and syscall protections unchanged.
    file(
        vfs,
        b"/run/systemd/system/systemd-logind.service.d/50-trames.conf",
        b"[Unit]\nConditionVirtualization=!container\n",
    )?;
    let fd = vfs.local_create(2 | trames_vfs::O_NONBLOCK | trames_vfs::O_CLOEXEC, 0)?;
    let socket = vfs.local_socket(fd)?.unwrap();
    socket.set_reuse(16, 1)?; // SO_PASSCRED: only PID 1 may announce boot readiness.
    vfs.local_bind(&socket, b"\x01\x00/run/host/notify\x00")?;
    // Retain the endpoint itself. PID 1 closes extra numeric FDs at startup.
    vfs.close(fd)?;
    Ok(Monitor {
        socket,
        signals_ready: false,
        ready: false,
        shutdown_sent: false,
    })
}

pub struct Monitor {
    socket: Rc<LocalSocket>,
    signals_ready: bool,
    ready: bool,
    shutdown_sent: bool,
}
impl Monitor {
    pub fn pending(&self, stopping: bool) -> bool {
        self.socket.poll(1) != 0 || (stopping && self.signals_ready && !self.shutdown_sent)
    }
    pub fn control(&mut self, stopping: bool) -> Result<Option<u32>> {
        while self.socket.poll(1) != 0 {
            let message = self.socket.recv(65536, 0x40)?;
            if message.credentials != Some([1, 0, 0]) {
                continue;
            }
            for field in message.data.split(|b| *b == b'\n') {
                if field == b"X_SYSTEMD_SIGNALS_LEVEL=2" {
                    self.signals_ready = true;
                }
                if field == b"READY=1" && !self.ready {
                    self.ready = true;
                    use std::io::IsTerminal;
                    eprintln!(
                        "trames: systemd ready{}",
                        if std::io::stdin().is_terminal() {
                            "\r"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
        if stopping && self.signals_ready && !self.shutdown_sent {
            self.shutdown_sent = true;
            // AArch64 Linux glibc SIGRTMIN (34) + 3. Never a host signal.
            return Ok(Some(37));
        }
        Ok(None)
    }
}
