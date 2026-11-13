// SPDX-License-Identifier: GPL-3.0-or-later
//! Cold host command jobs driven by existing embedding readiness. No I/O service.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
};
use trames_bridge::{Broker, HostRequest, Policy};
use trames_distro::environment::{self as model, Grant, Manifest};
struct Job {
    child: Child,
    output: [Option<std::fs::File>; 2],
    event: OwnedFd,
    pending: Option<Value>,
    status: Option<i32>,
    text_tail: [Vec<u8>; 2],
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.status.is_none() {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
        }
    }
}
pub struct Host {
    broker: Broker,
    jobs: BTreeMap<u64, Job>,
    trace: bool,
    wake: Option<Arc<trames_shared::HostWake>>,
    root: PathBuf,
    config: Manifest,
    grant: Grant,
    ports: Vec<model::Publish>,
    mounts: std::collections::BTreeSet<String>,
}
fn text_chunk(tail: &mut Vec<u8>, bytes: &[u8]) -> String {
    tail.extend_from_slice(bytes);
    let mut end = tail.len();
    for n in 1..=3.min(tail.len()) {
        if std::str::from_utf8(&tail[tail.len() - n..])
            .is_err_and(|e| e.valid_up_to() == 0 && e.error_len().is_none())
        {
            end = tail.len() - n;
            break;
        }
    }
    let text = String::from_utf8_lossy(&tail[..end]).into_owned();
    tail.drain(..end);
    text
}
fn ioe(e: impl ToString) -> io::Error {
    io::Error::other(e.to_string())
}
fn delegated(argv: &[String], uuid: &str) -> io::Result<Vec<String>> {
    let words: Vec<_> = argv.iter().map(String::as_str).collect();
    if matches!(
        words.as_slice(),
        ["--version"]
            | ["--list"]
            | ["--list", "--json"]
            | ["env", "ls"]
            | ["env", "ls", "--json"]
            | ["gui"]
    ) {
        return Ok(argv.to_vec());
    }
    if argv.iter().any(|v| v == "--global") {
        return Err(ioe("global host configuration is not delegated"));
    }
    let allowed = matches!(
        words.as_slice(),
        ["config", "show"]
            | ["config", "show", "--json"]
            | ["config", "get", _]
            | ["config", "path"]
            | ["config", "validate"]
            | ["session", "ls"]
            | ["session", "ls", "--json"]
            | ["env", "status"]
            | ["env", "start"]
            | ["env", "stop"]
            | ["env", "restart"]
    );
    if !allowed {
        return Err(ioe("host-cli permits read-only configuration and lifecycle of this environment; use the separately granted host-exec capability for host administration"));
    }
    let mut out = vec!["-e".into(), uuid.into()];
    out.extend_from_slice(argv);
    Ok(out)
}
unsafe extern "C" {
    fn trames_control_open_url(bytes: *const u8, length: usize) -> isize;
    fn trames_control_clipboard_read(
        board: *mut std::ffi::c_void,
        bytes: *mut u8,
        capacity: usize,
    ) -> isize;
    fn trames_control_clipboard_write(
        board: *mut std::ffi::c_void,
        bytes: *const u8,
        length: usize,
    ) -> isize;
}
fn clipboard(request: &trames_bridge::Request) -> Result<Value, String> {
    match request.method.as_str() {
        "clipboard.read" => {
            let mut bytes = vec![0; 16384];
            // SAFETY: only this owned output is writable for the call; no guest
            // address reaches AppKit. Null selects the system pasteboard.
            let count = unsafe {
                trames_control_clipboard_read(std::ptr::null_mut(), bytes.as_mut_ptr(), bytes.len())
            };
            if count < 0 {
                return Err(format!("clipboard read failed ({count})"));
            }
            bytes.truncate(count as usize);
            Ok(json!({"text":String::from_utf8(bytes).map_err(|e|e.to_string())?}))
        }
        "clipboard.write" => {
            let text = request.params["text"]
                .as_str()
                .ok_or("clipboard text is required")?;
            if text.len() > 16384 {
                return Err("clipboard text exceeds 16 KiB".into());
            }
            // SAFETY: the UTF-8 request owns exactly this readable byte range.
            let result = unsafe {
                trames_control_clipboard_write(std::ptr::null_mut(), text.as_ptr(), text.len())
            };
            if result < 0 {
                return Err(format!("clipboard write failed ({result})"));
            }
            Ok(json!({"written":true}))
        }
        _ => Err("unknown clipboard operation".into()),
    }
}
impl Host {
    pub fn new(root: PathBuf, config: Manifest, grant: Grant) -> io::Result<Self> {
        let broker = Broker::default();
        let e = grant.effective(&config.interop);
        // Capability admission is cold. File and packet operations retain their
        // original namespace/resources and worker readiness.
        broker
            .configure(
                Policy::new(
                    e.host_ui.enabled,
                    e.host_exec.enabled,
                    e.files.enabled,
                    e.ports.enabled && config.network.mode == model::NetworkMode::Bridge,
                    e.mounts.enabled,
                )
                .with_cli(e.host_cli.enabled)
                .with_clipboard(e.clipboard.enabled),
            )
            .map_err(ioe)?;
        Ok(Self {
            broker,
            jobs: BTreeMap::new(),
            trace: std::env::var_os("TRAMES_INTEROP_TRACE").is_some(),
            wake: None,
            root,
            ports: config.network.publish.clone(),
            mounts: config.mounts.iter().map(|m| m.target.clone()).collect(),
            config,
            grant,
        })
    }
    pub fn clipboard(&self) -> bool {
        self.broker.enabled(trames_bridge::Channel::Clipboard)
    }
    pub fn broker(&self) -> Broker {
        self.broker.clone()
    }
    pub fn bind(&mut self, wake: Arc<trames_shared::HostWake>) {
        self.wake = Some(wake.clone());
        if self.trace {
            eprintln!("interop bound host wake");
        }
        self.broker.bind_host_wake(wake);
    }
    pub fn interest(&self, wait: &mut trames_shared::HostWait) -> io::Result<()> {
        if self.jobs.is_empty() {
            return Ok(());
        }
        if let Some(deadline) = self.broker.deadline() {
            wait.until(deadline);
        }
        for job in self.jobs.values() {
            if job.pending.is_none() {
                for file in job.output.iter().flatten() {
                    wait.watch(file, false)?;
                }
                if job.status.is_none() {
                    wait.watch(&job.event, false)?;
                }
            }
        }
        Ok(())
    }
    fn spawn(&self, request: &HostRequest) -> io::Result<Job> {
        let argv: Vec<String> =
            serde_json::from_value(request.request.params["argv"].clone()).map_err(ioe)?;
        if argv.is_empty() || argv.iter().any(|v| v.contains('\0')) {
            return Err(ioe("argv must be a nonempty string array"));
        }
        let mut command = if request.request.method == "host.cli" {
            let mut c = Command::new(std::env::current_exe()?);
            c.env("TRAMES_HOME", &self.root);
            c.args(delegated(&argv, &self.config.environment.uuid)?);
            c
        } else {
            if !argv[0].starts_with('/') {
                return Err(ioe("host executable must use an absolute path"));
            }
            let mut c = Command::new(&argv[0]);
            c.args(&argv[1..]);
            c
        };
        if let Some(cwd) = request.request.params["cwd"].as_str() {
            command.current_dir(cwd);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let output = [
            child
                .stdout
                .take()
                .map(|f| std::fs::File::from(OwnedFd::from(f))),
            child
                .stderr
                .take()
                .map(|f| std::fs::File::from(OwnedFd::from(f))),
        ];
        for f in output.iter().flatten() {
            let flags = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETFL) };
            if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let event = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            let _ = child.kill();
            return Err(io::Error::last_os_error());
        }
        let change = libc::kevent {
            ident: child.id() as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let result =
            unsafe { libc::kevent(fd, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        let status = child
            .try_wait()?
            .map(|s| s.code().unwrap_or(128 + s.signal().unwrap_or(9)));
        if result < 0 && status.is_none() {
            let _ = child.kill();
            return Err(io::Error::last_os_error());
        }
        Ok(Job {
            child,
            output,
            event,
            pending: None,
            status,
            text_tail: [vec![], vec![]],
        })
    }
    fn control(
        &mut self,
        vfs: &trames_vfs::Vfs,
        request: &trames_bridge::Request,
    ) -> Result<Value, String> {
        let error = |e: trames_vfs::Errno| e.to_string();
        match request.method.as_str() {
            "ports.list" => Ok(json!({"publish":self.ports})),
            "ports.add" | "ports.remove" => {
                let host: std::net::SocketAddr = request.params["host"]
                    .as_str()
                    .ok_or("host socket address required")?
                    .parse()
                    .map_err(|e: std::net::AddrParseError| e.to_string())?;
                let protocol = request.params["protocol"].as_str().unwrap_or("tcp");
                let kind = match protocol {
                    "tcp" => 1,
                    "udp" => 2,
                    _ => return Err("protocol must be tcp or udp".into()),
                };
                if request.method == "ports.remove" {
                    if !self
                        .ports
                        .iter()
                        .any(|p| p.host == host && p.protocol == protocol)
                    {
                        return Err("published port does not belong to this environment".into());
                    }
                    vfs.unpublish_host_port(host, kind).map_err(error)?;
                    self.ports
                        .retain(|p| p.host != host || p.protocol != protocol);
                } else {
                    let guest = request.params["guest"]
                        .as_u64()
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|p| *p > 0)
                        .ok_or("guest port required")?;
                    vfs.publish_host_port(host, guest, kind).map_err(error)?;
                    self.ports.push(model::Publish {
                        protocol: protocol.into(),
                        host,
                        guest,
                    });
                }
                Ok(json!({"updated":true,"persistent":false}))
            }
            "mounts.list" => Ok(json!({"targets":self.mounts,"transport":"hostfs"})),
            "mounts.add" | "mounts.remove" => {
                let target = request.params["target"]
                    .as_str()
                    .filter(|p| p.starts_with('/'))
                    .ok_or("absolute guest target required")?;
                if request.method == "mounts.remove" {
                    if !self.mounts.contains(target) {
                        return Err("mount is not owned by this environment control".into());
                    }
                    vfs.unmount_host_authorized(target.as_bytes())
                        .map_err(error)?;
                    self.mounts.remove(target);
                } else {
                    let alias = request.params["share"]
                        .as_str()
                        .ok_or("authorized share alias required")?;
                    let effective = self.grant.effective(&self.config.interop);
                    if !effective.mounts.shares.iter().any(|v| v == alias) {
                        return Err("share is outside the requested mount grant".into());
                    }
                    let share = self.grant.shares.get(alias).ok_or("unknown host share")?;
                    let readonly =
                        share.readonly || request.params["readonly"].as_bool().unwrap_or(false);
                    vfs.mount_host_authorized(target.as_bytes(), &share.host, readonly)
                        .map_err(error)?;
                    self.mounts.insert(target.into());
                }
                Ok(json!({"updated":true,"persistent":false}))
            }
            _ => Err("unsupported control operation".into()),
        }
    }
    pub fn poll(&mut self, vfs: &trames_vfs::Vfs) -> io::Result<()> {
        if self.jobs.is_empty() && !self.broker.host_ready() {
            return Ok(());
        }
        for _ in 0..16 {
            let Some(bytes) = self.broker.poll(trames_bridge::MAX_FRAME).map_err(ioe)? else {
                break;
            };
            let request: HostRequest = serde_json::from_slice(&bytes).map_err(ioe)?;
            if self.trace {
                eprintln!(
                    "interop request {} {}",
                    request.ticket, request.request.method
                );
            }
            if !self.broker.active(request.ticket) {
                continue;
            }
            let result = match request.request.method.as_str() {
                "ports.list" | "ports.add" | "ports.remove" | "mounts.list" | "mounts.add"
                | "mounts.remove" => self.control(vfs, &request.request),
                "clipboard.read" | "clipboard.write" => clipboard(&request.request),
                "url.open" => {
                    match request.request.params["url"].as_str() {
                        Some(url) => {
                            // SAFETY: host-owned UTF-8 bytes, copied by the native adapter.
                            let result =
                                unsafe { trames_control_open_url(url.as_ptr(), url.len()) };
                            if result < 0 {
                                Err("only HTTP(S) host URLs are supported".into())
                            } else {
                                Ok(json!({"opened":result!=0}))
                            }
                        }
                        None => Err("URL is required".into()),
                    }
                }
                "host.exec" | "host.cli" => match self.spawn(&request) {
                    Ok(job) => {
                        if self.trace {
                            eprintln!("interop spawned {} status {:?}", job.child.id(), job.status);
                        }
                        self.jobs.insert(request.ticket, job);
                        // Interest was armed before this deciding pass. The
                        // new pipe/process FDs must be armed on the next pass.
                        if let Some(wake) = &self.wake {
                            wake.notify();
                        }
                        continue;
                    }
                    Err(e) => Err(e.to_string()),
                },
                "device.info" => {
                    Ok(json!({"platform":"macos","environment":self.config.environment}))
                }
                "files.list" => Ok(
                    json!({"shares":self.grant.effective(&self.config.interop).files.shares,"mounts":self.config.mounts,"transport":"hostfs"}),
                ),
                _ => Err("unsupported host control operation".into()),
            };
            let reply = match result {
                Ok(value) => json!({"version":1,"ticket":request.ticket,"result":value}),
                Err(message) => {
                    json!({"version":1,"ticket":request.ticket,"error":{"code":"host_error","message":message}})
                }
            };
            let mut bytes = serde_json::to_vec(&reply).map_err(ioe)?;
            if bytes.len() > trames_bridge::MAX_FRAME {
                bytes=serde_json::to_vec(&json!({"version":1,"ticket":request.ticket,"error":{"code":"too_large","message":"Host reply exceeds control frame size"}})).map_err(ioe)?;
            }
            if let Err(code) = self.broker.reply(&bytes) {
                if code != 2 {
                    return Err(ioe(code));
                }
            }
        }
        let mut done = vec![];
        for (&ticket, job) in &mut self.jobs {
            if !self.broker.active(ticket) {
                if job.status.is_none() {
                    unsafe {
                        libc::kill(-(job.child.id() as i32), libc::SIGKILL);
                    }
                }
                job.status = job
                    .child
                    .try_wait()?
                    .map(|s| s.code().unwrap_or(128 + s.signal().unwrap_or(9)));
                if job.status.is_some() {
                    done.push(ticket);
                }
                continue;
            }
            if let Some(value) = job.pending.take() {
                match self.broker.stream(ticket, value.clone(), false) {
                    Ok(()) => (),
                    Err(11) => {
                        job.pending = Some(value);
                        continue;
                    }
                    Err(2) => continue,
                    Err(e) => return Err(ioe(e)),
                }
            }
            for (direction, file) in job.output.iter_mut().enumerate() {
                if let Some(f) = file {
                    let mut bytes = [0; 2048];
                    match f.read(&mut bytes) {
                        Ok(0) => {
                            if self.trace {
                                eprintln!("interop EOF {ticket} {direction}");
                            }
                            *file = None;
                            if !job.text_tail[direction].is_empty() {
                                let field = if direction == 0 { "stdout" } else { "stderr" };
                                let text = String::from_utf8_lossy(&std::mem::take(
                                    &mut job.text_tail[direction],
                                ))
                                .into_owned();
                                let value = json!({field:text,"done":false});
                                match self.broker.stream(ticket, value.clone(), false) {
                                    Ok(()) => (),
                                    Err(11) => {
                                        job.pending = Some(value);
                                        break;
                                    }
                                    Err(2) => (),
                                    Err(e) => return Err(ioe(e)),
                                }
                            }
                        }
                        Ok(n) => {
                            if self.trace {
                                eprintln!("interop bytes {ticket} {direction} {n}");
                            }
                            let field = if direction == 0 { "stdout" } else { "stderr" };
                            let text = text_chunk(&mut job.text_tail[direction], &bytes[..n]);
                            if text.is_empty() {
                                continue;
                            }
                            let value = json!({field:text,"done":false});
                            match self.broker.stream(ticket, value.clone(), false) {
                                Ok(()) => (),
                                Err(11) => {
                                    job.pending = Some(value);
                                    break;
                                }
                                Err(2) => break,
                                Err(e) => return Err(ioe(e)),
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Err(e) => return Err(e),
                    }
                }
            }
            if job.status.is_none() {
                job.status = job
                    .child
                    .try_wait()?
                    .map(|s| s.code().unwrap_or(128 + s.signal().unwrap_or(9)));
            }
            if job.output.iter().all(Option::is_none) && job.pending.is_none() {
                if let Some(code) = job.status {
                    match self
                        .broker
                        .stream(ticket, json!({"done":true,"exit_code":code}), true)
                    {
                        Ok(()) | Err(2) => done.push(ticket),
                        Err(11) => (),
                        Err(e) => return Err(ioe(e)),
                    }
                }
            }
        }
        for ticket in done {
            self.jobs.remove(&ticket);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_jobs_rearm_original_waiter_and_stream_until_exit_without_polling() {
        let mut config = Manifest::default();
        config.interop.host_exec.enabled = true;
        let mut host = Host::new(
            std::env::temp_dir(),
            config,
            Grant {
                host_exec: true,
                ..Default::default()
            },
        )
        .unwrap();
        let connection = host
            .broker
            .connect_channel(trames_bridge::Channel::HostExec)
            .unwrap();
        let mut wait = trames_shared::HostWait::new().unwrap();
        host.bind(wait.wake());
        connection.write(b"{\"version\":1,\"id\":1,\"method\":\"host.exec\",\"params\":{\"argv\":[\"/bin/echo\",\"original-waiter\"]}}\n").unwrap();
        let vfs = trames_vfs::Vfs::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut output = String::new();
        loop {
            wait.begin();
            host.interest(&mut wait).unwrap();
            wait.until(deadline);
            host.poll(&vfs).unwrap();
            match connection.read(32768) {
                Ok(bytes) => output.push_str(std::str::from_utf8(&bytes).unwrap()),
                Err(11) => (),
                other => panic!("unexpected reply {other:?}"),
            }
            if output.contains("\"done\":true") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "lost source-set change: {output}"
            );
            wait.wait().unwrap();
        }
        assert!(output.contains("original-waiter"));
        assert!(output.contains("\"exit_code\":0"));
        assert!(host.jobs.is_empty());
    }
    #[test]
    fn delegated_cli_cannot_change_grants_or_select_another_environment() {
        let args = |s: &[&str]| s.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(delegated(&args(&["env", "grant", "other", "host-exec", "on"]), "own").is_err());
        assert!(delegated(&args(&["-e", "other", "exec", "--", "sh"]), "own").is_err());
        assert!(delegated(&args(&["debug", "run", "--mount", "/=host"]), "own").is_err());
        assert_eq!(
            delegated(&args(&["config", "show"]), "own").unwrap(),
            args(&["-e", "own", "config", "show"])
        );
    }
    #[test]
    fn stream_frames_preserve_unicode_across_pipe_boundaries() {
        let text = "a中文 🧪 b";
        for size in 1..=5 {
            let mut tail = vec![];
            let mut result = String::new();
            for bytes in text.as_bytes().chunks(size) {
                result.push_str(&text_chunk(&mut tail, bytes));
            }
            assert!(tail.is_empty());
            assert_eq!(result, text);
        }
    }
}
