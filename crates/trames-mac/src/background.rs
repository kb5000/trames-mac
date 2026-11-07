// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-environment launchd lifetime. This path never handles guest syscalls or I/O.
use super::environments::{self, Result};
use std::{
    fs, io,
    os::unix::fs::OpenOptionsExt,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
fn label(root: &Path, name: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    let hash = format!("{:x}", Sha256::digest(root.as_os_str().as_bytes()));
    format!(
        "org.trames.{}.env.{}.{name}",
        super::product::CHANNEL,
        &hash[..16]
    )
}
fn domain() -> String {
    format!("gui/{}", unsafe { libc::geteuid() })
}
fn target(root: &Path, name: &str) -> String {
    format!("{}/{}", domain(), label(root, name))
}
fn agent(root: &Path, name: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(root, name))))
}
fn xml(s: &str) -> Result<String> {
    if s.chars()
        .any(|c| c < ' ' && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err("launchd path contains an XML control character".into());
    }
    Ok(s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
fn plist(root: &Path, name: &str) -> Result<String> {
    let exe = std::env::current_exe()?;
    let error = root.join("logs").join(format!("{name}.log"));
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>env</string><string>_serve</string><string>{}</string></array>\n<key>EnvironmentVariables</key><dict><key>TRAMES_HOME</key><string>{}</string></dict>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n<key>ThrottleInterval</key><integer>5</integer>\n<key>ExitTimeOut</key><integer>90</integer>\n<key>Umask</key><integer>63</integer>\n<key>StandardOutPath</key><string>{}</string>\n<key>StandardErrorPath</key><string>{}</string>\n</dict></plist>\n",xml(&label(root,name))?,xml(exe.to_str().ok_or("executable path is not UTF-8")?)?,xml(name)?,xml(root.to_str().ok_or("registry path is not UTF-8")?)?,xml(error.to_str().ok_or("log path is not UTF-8")?)?,xml(error.to_str().unwrap())?))
}
// Socket pathnames survive a server exit. A live listener, rather than a stale
// inode, is the readiness boundary for management clients; no guest is created.
fn console_ready(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}
fn loaded(root: &Path, name: &str) -> bool {
    Command::new("/bin/launchctl")
        .args(["print", &target(root, name)])
        .output()
        .is_ok_and(|r| r.status.success())
}
fn ctl(args: &[&str]) -> Result<()> {
    let r = Command::new("/bin/launchctl").args(args).output()?;
    if !r.status.success() {
        return Err(format!(
            "launchctl {} failed: {}",
            args[0],
            String::from_utf8_lossy(&r.stderr)
        )
        .into());
    }
    Ok(())
}
fn ensure_dirs(root: &Path) -> Result<()> {
    environments::private_dir(&root.join("services"))?;
    environments::private_dir(&root.join("logs"))?;
    Ok(())
}
pub fn registered(root: &Path, name: &str) -> bool {
    root.join("services")
        .join(format!("{name}.plist"))
        .is_file()
}
pub fn start(root: &Path, name: &str) -> Result<()> {
    environments::load(&environments::directory(root, name)?)?;
    // Management only: serialize concurrent starts of this environment. This
    // file is distinct from the guest owner's lifetime lock, and is never
    // held by a running guest or used on any guest I/O path.
    let guard = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("locks").join(format!(".background-{name}")))?;
    guard.lock()?;
    if environments::running(root, name) {
        if console_ready(&environments::console_socket(root, name)?) {
            return Ok(());
        }
        if !loaded(root, name) {
            return Err("environment has a foreground/maintenance owner; stop it before starting a managed background session".into());
        }
        // A login-started server can hold its original owner lock before it
        // has rebound/listened on the console. Reuse startup's bounded wait.
    }
    ensure_dirs(root)?;
    let log = root.join("logs").join(format!("{name}.log"));
    if fs::metadata(&log).is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
        let _ = fs::remove_file(log.with_extension("log.1"));
        fs::rename(&log, log.with_extension("log.1"))?;
    }
    let text = plist(root, name)?;
    let job = root.join("services").join(format!("{name}.plist"));
    environments::atomic_write(&job, text.as_bytes())?;
    let autostart = agent(root, name)?;
    if autostart.is_file() {
        environments::atomic_write(&autostart, text.as_bytes())?;
    }
    if !loaded(root, name) {
        ctl(&[
            "bootstrap",
            &domain(),
            job.to_str().ok_or("job path is not UTF-8")?,
        ])?;
    } else {
        ctl(&["kickstart", &target(root, name)])?;
    }
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if environments::running(root, name)
            && console_ready(&environments::console_socket(root, name)?)
        {
            eprintln!("Started {name} in background");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = unload(root, name);
    Err(format!(
        "background startup did not become ready; inspect {}",
        log.display()
    )
    .into())
}
pub fn serve(root: &Path, name: &str) -> Result<u8> {
    let path = environments::console_socket(root, name)?;
    environments::start(
        root,
        name,
        &[
            "--console-socket".into(),
            path.to_string_lossy().into_owned(),
        ],
    )
}
pub fn attach(root: &Path, name: &str, args: &[String]) -> Result<u8> {
    environments::load(&environments::directory(root, name)?)?;
    let id = match args {
        [] => 0,
        [flag, id] if flag == "--terminal" => id.parse()?,
        _ => return Err("usage: env attach NAME [--terminal ID]".into()),
    };
    super::console::attach(&environments::console_socket(root, name)?, id)?;
    Ok(0)
}
pub fn autostart(root: &Path, name: &str, value: &str) -> Result<()> {
    environments::load(&environments::directory(root, name)?)?;
    let path = agent(root, name)?;
    match value {
        "on" => {
            ensure_dirs(root)?;
            environments::private_dir(path.parent().unwrap())?;
            environments::atomic_write(&path, plist(root, name)?.as_bytes())?;
            println!("Enabled {name} at Mac user login; env start {name} --detach starts it now");
        }
        "off" => {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            println!("Disabled {name} at login; current environment lifetime is unchanged");
        }
        _ => return Err("usage: env autostart NAME on|off".into()),
    }
    Ok(())
}
pub fn unload(root: &Path, name: &str) -> Result<()> {
    if loaded(root, name) {
        ctl(&["bootout", &target(root, name)])?;
    }
    Ok(())
}
pub fn status(root: &Path, name: &str) -> Result<()> {
    environments::load(&environments::directory(root, name)?)?;
    println!(
        "{}",
        serde_json::json!({"running":environments::running(root,name),"background_registered":registered(root,name),"autostart":agent(root,name)?.is_file(),"console":environments::console_socket(root,name)?,"log":root.join("logs").join(format!("{name}.log"))})
    );
    Ok(())
}
pub fn remove(root: &Path, name: &str) -> Result<()> {
    if registered(root, name) {
        unload(root, name)?;
    }
    for path in [
        agent(root, name)?,
        root.join("services").join(format!("{name}.plist")),
    ] {
        match fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_console_readiness_rejects_a_stale_socket_path() {
        let path = PathBuf::from("/tmp").join(format!(
            "tr-bg-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(console_ready(&path));
        drop(listener);
        assert!(path.exists());
        assert!(!console_ready(&path));
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn registry_and_channel_are_part_of_job_identity_and_paths_are_xml_escaped() {
        let a = Path::new("/tmp/stable space");
        let b = Path::new("/tmp/dev");
        assert_ne!(label(a, "docker"), label(b, "docker"));
        assert_ne!(label(a, "docker"), label(a, "other"));
        assert_eq!(xml("a&b<\"'>").unwrap(), "a&amp;b&lt;&quot;&apos;&gt;");
        assert!(xml("bad\0path").is_err());
        let text = plist(a, "docker").unwrap();
        assert!(text.contains("<string>_serve</string>"));
        assert!(text.contains("<key>ExitTimeOut</key><integer>90</integer>"));
        assert!(!text.contains("/bin/sh"));
    }
}
