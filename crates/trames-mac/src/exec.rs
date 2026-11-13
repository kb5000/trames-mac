// SPDX-License-Identifier: GPL-3.0-or-later
//! Structured host commands use original process/pipe/PTY resources while CPUs run.
//! Accepted image loading and task admission use existing runtime workers.
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use trames_linux::{HostProcess, HostSpawn, HOST_RUNNING};
use trames_vfs::{DirectFd, Vfs};
const MAGIC: &[u8; 8] = b"TRMEXEC1";
const LIMIT: usize = 65536;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub argv: Vec<String>,
    pub environment: Vec<String>,
    pub cwd: Option<String>,
    pub tty: bool,
    pub size: Option<(u16, u16)>,
}
struct Job {
    input: Option<DirectFd>,
    output: [Option<DirectFd>; 2],
    terminal: Option<Arc<trames_vfs::Pty>>,
    process: Arc<HostProcess>,
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.process.status() == HOST_RUNNING {
            self.process.cancel();
        }
    }
}
struct Peer {
    socket: UnixStream,
    incoming: Vec<u8>,
    outgoing: VecDeque<u8>,
    request: Option<Request>,
    job: Option<Job>,
    stdin: VecDeque<u8>,
    eof: bool,
    done: bool,
}
impl Peer {
    fn frame(&mut self, kind: u8, bytes: &[u8]) {
        self.outgoing.push_back(kind);
        self.outgoing.extend((bytes.len() as u32).to_be_bytes());
        self.outgoing.extend(bytes);
    }
    fn flush(&mut self) -> bool {
        while !self.outgoing.is_empty() {
            match self.socket.write(self.outgoing.as_slices().0) {
                Ok(0) => return false,
                Ok(n) => {
                    self.outgoing.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        !self.done || !self.outgoing.is_empty()
    }
}
pub fn path(console: &Path) -> PathBuf {
    console.with_extension("exec")
}
pub struct Server {
    wake: Option<Arc<trames_linux::HostWake>>,
    listener: UnixListener,
    path: PathBuf,
    peers: Vec<Peer>,
    launches: VecDeque<HostSpawn>,
    environment: Vec<String>,
}
impl Server {
    pub fn bind_wake(&mut self, wake: Arc<trames_linux::HostWake>) {
        self.wake = Some(wake);
    }
    pub fn interest(&self, wait: &mut trames_linux::HostWait) -> io::Result<()> {
        wait.watch(&self.listener, false)?;
        for peer in &self.peers {
            if !peer.done && peer.incoming.len() < LIMIT + 12 && peer.stdin.len() < LIMIT {
                wait.watch(&peer.socket, false)?;
            }
            if !peer.outgoing.is_empty() {
                wait.watch(&peer.socket, true)?;
            }
            if let Some(job) = &peer.job {
                wait.retain([job.process.completion().subscribe(wait.signal())]);
                if !peer.stdin.is_empty() {
                    if let Some(fd) = &job.input {
                        wait.retain(fd.subscribe(wait.signal()));
                    }
                }
                if peer.outgoing.len() <= LIMIT {
                    for fd in job.output.iter().flatten() {
                        wait.retain(fd.subscribe(wait.signal()));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn bind(console: &Path, environment: Vec<String>) -> io::Result<Self> {
        let path = path(console);
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.file_type().is_socket() => {
                return Err(io::Error::other("exec path is not a socket"))
            }
            Ok(_) => fs::remove_file(&path)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            wake: None,
            listener,
            path,
            peers: vec![],
            launches: VecDeque::new(),
            environment,
        })
    }
    pub fn spawn(&mut self) -> Option<HostSpawn> {
        self.launches.pop_front()
    }
    pub fn poll(&mut self) -> io::Result<()> {
        match self.listener.accept() {
            Ok((socket, _)) if self.peers.len() < 32 => {
                socket.set_nonblocking(true)?;
                if let Some(wake) = &self.wake {
                    wake.notify();
                }
                self.peers.push(Peer {
                    socket,
                    incoming: vec![],
                    outgoing: VecDeque::new(),
                    request: None,
                    job: None,
                    stdin: VecDeque::new(),
                    eof: false,
                    done: false,
                });
            }
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e),
        }
        for mut p in std::mem::take(&mut self.peers) {
            let had_output = !p.outgoing.is_empty();
            let had_input = !p.stdin.is_empty();
            let mut alive = p.flush();
            if alive && !p.done && p.incoming.len() < LIMIT + 12 && p.stdin.len() < LIMIT {
                let mut bytes = [0; 4096];
                match p.socket.read(&mut bytes) {
                    Ok(0) => alive = false,
                    Ok(n) => p.incoming.extend_from_slice(&bytes[..n]),
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => alive = false,
                }
            }
            if alive && p.request.is_none() && p.incoming.len() >= 12 {
                let n = u32::from_be_bytes(p.incoming[8..12].try_into().unwrap()) as usize;
                if &p.incoming[..8] != MAGIC || n > LIMIT {
                    alive = false;
                } else if p.incoming.len() >= n + 12 {
                    match serde_json::from_slice::<Request>(&p.incoming[12..12 + n]) {
                        Ok(r)
                            if !r.argv.is_empty()
                                && r.argv
                                    .iter()
                                    .chain(&r.environment)
                                    .all(|s| !s.contains('\0')) =>
                        {
                            p.request = Some(r)
                        }
                        _ => alive = false,
                    }
                    p.incoming.drain(..12 + n);
                }
            }
            while alive && p.request.is_some() && p.incoming.len() >= 5 {
                let kind = p.incoming[0];
                let n = u32::from_be_bytes(p.incoming[1..5].try_into().unwrap()) as usize;
                if n > 4096 || !matches!((kind, n), (0, _) | (1, 0) | (2, 1) | (3, 4)) {
                    alive = false;
                    break;
                }
                if p.incoming.len() < n + 5 || (kind == 0 && p.stdin.len() + n > LIMIT) {
                    break;
                }
                match kind {
                    0 if !p.eof => p.stdin.extend(&p.incoming[5..5 + n]),
                    1 => p.eof = true,
                    2 => {
                        if let Some(job) = &p.job {
                            let signal = p.incoming[5] as u32;
                            job.process.signal(signal);
                        }
                    }
                    3 => {
                        if let Some(pair) = p.job.as_ref().and_then(|j| j.terminal.as_ref()) {
                            pair.resize_from_host(
                                u16::from_be_bytes(p.incoming[5..7].try_into().unwrap()),
                                u16::from_be_bytes(p.incoming[7..9].try_into().unwrap()),
                            );
                        }
                    }
                    _ => (),
                }
                p.incoming.drain(..n + 5);
            }
            if alive {
                Self::pump(&mut p);
                alive = p.flush();
            }
            if alive {
                if !had_output && !p.outgoing.is_empty() || !had_input && !p.stdin.is_empty() {
                    if let Some(wake) = &self.wake {
                        wake.notify();
                    }
                }
                self.peers.push(p);
            }
        }
        Ok(())
    }
    fn pump(p: &mut Peer) {
        let Some(job) = &mut p.job else {
            return;
        };
        if let Some(input) = &job.input {
            if !p.stdin.is_empty() {
                let result = if let Some(pair) = &job.terminal {
                    pair.master_write(p.stdin.as_slices().0)
                        .map(|(n, signals)| {
                            pair.queue_host_signals(signals);
                            n
                        })
                } else {
                    input.write(p.stdin.as_slices().0)
                };
                match result {
                    Ok(n) => {
                        p.stdin.drain(..n);
                    }
                    Err(e) if e.0 == 11 => (),
                    Err(_) => {
                        p.stdin.clear();
                        p.eof = true;
                    }
                }
            }
            if p.eof && p.stdin.is_empty() {
                if let Some(pair) = &job.terminal {
                    let _ = pair.master_write(&[4]);
                }
                job.input = None;
            }
        }
        if let Some(pair) = &job.terminal {
            pair.flush_echo();
        }
        for (index, output) in job.output.iter_mut().enumerate() {
            if p.outgoing.len() > LIMIT {
                break;
            }
            if let Some(fd) = output.as_ref() {
                match fd.read(4096) {
                    Ok(bytes) if bytes.is_empty() => *output = None,
                    Ok(bytes) => {
                        p.outgoing.push_back(index as u8 + 1);
                        p.outgoing.extend((bytes.len() as u32).to_be_bytes());
                        p.outgoing.extend(bytes);
                    }
                    Err(e) if e.0 == 11 => (),
                    Err(_) => *output = None,
                }
            }
        }
        let status = job.process.status();
        if status != HOST_RUNNING
            && (status < 0 || job.output.iter().all(Option::is_none))
            && !p.done
        {
            if status < 0 {
                p.frame(
                    4,
                    format!("cannot execute command: Linux errno {}", -status).as_bytes(),
                );
            }
            p.frame(3, &status.to_be_bytes());
            p.done = true;
        }
    }
    pub fn control(&mut self, root: &Vfs) {
        for p in &mut self.peers {
            if p.job.is_some() || p.done {
                continue;
            }
            let Some(r) = &p.request else {
                continue;
            };
            match prepare(root, r, &self.environment) {
                Ok((job, launch)) => {
                    p.job = Some(job);
                    self.launches.push_back(launch);
                }
                Err(e) => {
                    p.frame(4, e.to_string().as_bytes());
                    p.frame(3, &(-22i32).to_be_bytes());
                    p.done = true;
                }
            }
            // This request has just created its original I/O/completion sources
            // (or an error response). Subscribe and retry them before sleeping.
            if let Some(wake) = &self.wake {
                wake.notify();
            }
        }
    }
}
fn prepare(
    root: &Vfs,
    r: &Request,
    base: &[String],
) -> Result<(Job, HostSpawn), Box<dyn std::error::Error>> {
    let mut host = root.clone();
    // This is a new host transport, not close(2) by PID 1. Discard only the
    // copied table view; closing it under PID 1 would release its POSIX locks.
    host.detach_file_table();
    host.set_lock_owner(0);
    host.set_controlling_terminal(None)?;
    let (mut child, input, output, terminal) = if r.tty {
        let fd = host.open_at(
            trames_vfs::AT_FDCWD,
            b"/dev/ptmx",
            2 | trames_vfs::O_NONBLOCK,
            0,
        )?;
        let pair = host.pty(fd)?.ok_or("PTY unavailable")?.pair.clone();
        pair.locked.store(false, Ordering::Relaxed);
        let (rows, cols) = r.size.unwrap_or((24, 80));
        pair.resize_from_host(rows, cols);
        let mut child = host.clone();
        child.close_all();
        let slave = format!("/dev/pts/{}", pair.number);
        let fd = child.open_at(trames_vfs::AT_FDCWD, slave.as_bytes(), 2 | 0x100, 0)?;
        if fd != 0 {
            child.dup3(fd, 0, 0)?;
        }
        child.dup3(0, 1, 0)?;
        child.dup3(0, 2, 0)?;
        let master = retain(&host, 0)?;
        (child, master.clone(), [Some(master), None], Some(pair))
    } else {
        let [stdin, input] = host.pipe(0)?;
        let [output, stdout] = host.pipe(0)?;
        let [error, stderr] = host.pipe(0)?;
        let input = retain(&host, input)?;
        let output = [Some(retain(&host, output)?), Some(retain(&host, error)?)];
        let mut child = host.clone();
        if stdin != 0 {
            child.dup3(stdin, 0, 0)?;
        }
        child.dup3(stdout, 1, 0)?;
        child.dup3(stderr, 2, 0)?;
        for fd in 3..6 {
            child.close(fd)?;
        }
        (child, input, output, None)
    };
    host.close_all();
    if let Some(cwd) = &r.cwd {
        child.chdir(cwd.as_bytes())?;
    }
    let mut environment = base.to_vec();
    for entry in &r.environment {
        let (key, _) = entry
            .split_once('=')
            .filter(|(key, _)| !key.is_empty())
            .ok_or("--setenv requires KEY=VALUE")?;
        environment.retain(|e| e.split_once('=').is_none_or(|(k, _)| k != key));
        environment.push(entry.clone());
    }
    let process = Arc::new(HostProcess::default());
    let launch = HostSpawn {
        argv: r.argv.iter().map(|s| s.as_bytes().to_vec()).collect(),
        environment: environment.iter().map(|s| s.as_bytes().to_vec()).collect(),
        vfs: child,
        process: process.clone(),
    };
    Ok((
        Job {
            input: Some(input),
            output,
            terminal,
            process,
        },
        launch,
    ))
}
fn retain(vfs: &Vfs, fd: u32) -> Result<DirectFd, Box<dyn std::error::Error>> {
    vfs.shared_files()
        .lookup(fd)
        .ok_or("descriptor not shared")??
        .direct_byte()
        .ok_or_else(|| "descriptor is not a byte resource".into())
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn queue(outgoing: &mut VecDeque<u8>, kind: u8, bytes: &[u8]) {
    outgoing.push_back(kind);
    outgoing.extend((bytes.len() as u32).to_be_bytes());
    outgoing.extend(bytes);
}
pub fn run(console: &Path, request: Request) -> io::Result<u8> {
    run_input(console, request, io::stdin())
}
pub fn run_input(console: &Path, request: Request, input: impl Read + AsRawFd) -> io::Result<u8> {
    run_streams(
        console,
        request,
        input,
        &mut io::stdout(),
        &mut io::stderr(),
    )
}
pub fn capture(console: &Path, request: Request) -> io::Result<(u8, Vec<u8>, Vec<u8>)> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = run_streams(
        console,
        request,
        fs::File::open("/dev/null")?,
        &mut stdout,
        &mut stderr,
    )?;
    Ok((code, stdout, stderr))
}
fn run_streams(
    console: &Path,
    request: Request,
    mut input: impl Read + AsRawFd,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<u8> {
    let mut socket = UnixStream::connect(path(console)).map_err(|e| io::Error::new(e.kind(), format!("exec service unavailable ({e}); restart this environment to load the updated runtime")))?;
    socket.set_write_timeout(Some(Duration::from_secs(30)))?;
    let bytes = serde_json::to_vec(&request)?;
    if bytes.len() > LIMIT {
        return Err(io::Error::other("command arguments exceed 64 KiB"));
    }
    socket.write_all(MAGIC)?;
    socket.write_all(&(bytes.len() as u32).to_be_bytes())?;
    socket.write_all(&bytes)?;
    let _signals = super::lifecycle::Signals::install()?;
    let _raw = super::session::RawTerminal::enter(request.tty)?;
    socket.set_nonblocking(true)?;
    let mut outgoing = VecDeque::new();
    let mut input_open = true;
    let mut incoming = Vec::new();
    let mut size = request.size;
    loop {
        if let Some(signal) = super::lifecycle::take_signal() {
            queue(&mut outgoing, 2, &[signal as u8]);
        }
        if request.tty {
            let now = super::session::window_size();
            if now != size {
                size = now;
                if let Some((r, c)) = size {
                    queue(
                        &mut outgoing,
                        3,
                        &[r.to_be_bytes(), c.to_be_bytes()].concat(),
                    );
                }
            }
        }
        let mut fds = [
            libc::pollfd {
                fd: socket.as_raw_fd(),
                events: libc::POLLIN
                    | if outgoing.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                revents: 0,
            },
            libc::pollfd {
                fd: if input_open && outgoing.len() < LIMIT {
                    input.as_raw_fd()
                } else {
                    -1
                },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, 100) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        let mut bytes = [0; 4096];
        if fds[0].revents & libc::POLLOUT != 0 && !outgoing.is_empty() {
            match socket.write(outgoing.as_slices().0) {
                Ok(n) => {
                    outgoing.drain(..n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            let n = match socket.read(&mut bytes) {
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            };
            if n == 0 {
                return Err(io::Error::other(
                    "exec connection closed before exit status",
                ));
            }
            incoming.extend_from_slice(&bytes[..n]);
            while incoming.len() >= 5 {
                let kind = incoming[0];
                let n = u32::from_be_bytes(incoming[1..5].try_into().unwrap()) as usize;
                if n > LIMIT {
                    return Err(io::Error::other("invalid exec response"));
                }
                if incoming.len() < n + 5 {
                    break;
                }
                match kind {
                    1 => {
                        stdout.write_all(&incoming[5..5 + n])?;
                        stdout.flush()?;
                    }
                    2 => {
                        stderr.write_all(&incoming[5..5 + n])?;
                        stderr.flush()?;
                    }
                    4 => eprintln!("trames: {}", String::from_utf8_lossy(&incoming[5..5 + n])),
                    3 if n == 4 => {
                        let status = i32::from_be_bytes(incoming[5..9].try_into().unwrap());
                        return Ok(if status == -2 || status == -20 {
                            127
                        } else if status < 0 {
                            126
                        } else if status & 127 == 0 {
                            (status >> 8) as u8
                        } else {
                            (128 + (status & 127)) as u8
                        });
                    }
                    _ => return Err(io::Error::other("invalid exec response kind")),
                }
                incoming.drain(..n + 5);
            }
        }
        if fds[1].revents != 0 {
            let n = input.read(&mut bytes)?;
            queue(&mut outgoing, if n == 0 { 1 } else { 0 }, &bytes[..n]);
            if n == 0 {
                input_open = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_request_admission_notifies_before_new_sources_or_responses_are_waited_on() {
        for invalid_cwd in [false, true] {
            let console = std::env::temp_dir().join(format!(
                "trames-late-exec-{}-{}-{invalid_cwd}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let mut server = Server::bind(&console, vec!["PATH=/bin".into()]).unwrap();
            let mut wait = trames_linux::HostWait::new().unwrap();
            server.bind_wake(wait.wake());
            let (socket, _client) = UnixStream::pair().unwrap();
            socket.set_nonblocking(true).unwrap();
            server.peers.push(Peer {
                socket,
                incoming: Vec::new(),
                outgoing: VecDeque::new(),
                request: Some(Request {
                    argv: vec!["cat".into()],
                    cwd: invalid_cwd.then(|| "/missing-directory".into()),
                    ..Default::default()
                }),
                job: None,
                stdin: VecDeque::new(),
                eof: false,
                done: false,
            });
            wait.begin();
            server.interest(&mut wait).unwrap();
            server.control(&Vfs::default());
            assert_eq!(server.peers[0].job.is_some(), !invalid_cwd);
            let start = std::time::Instant::now();
            wait.until(start + Duration::from_secs(2));
            wait.wait().unwrap();
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "new sources/response did not notify the embedding"
            );
        }
    }
    #[test]
    fn launch_transport_preserves_init_locks_and_owns_independent_binary_streams() {
        let mut root = Vfs::default();
        root.set_lock_owner(7);
        let file = root
            .open_at(trames_vfs::AT_FDCWD, b"/init-lock", 0x42, 0o600)
            .unwrap();
        let lock = root.normalize_lock(file, 1, 0, 0, 8).unwrap();
        root.record_lock_set(file, lock).unwrap();
        let mut contender = root.clone();
        contender.set_lock_owner(8);
        let wanted = contender.normalize_lock(file, 1, 0, 0, 8).unwrap();
        assert!(contender
            .record_lock_conflict(file, wanted)
            .unwrap()
            .is_some());
        let request = Request {
            argv: vec!["cat".into()],
            ..Default::default()
        };
        let (mut job, mut launch) = prepare(&root, &request, &["PATH=/bin".into()]).unwrap();
        assert!(contender
            .record_lock_conflict(file, wanted)
            .unwrap()
            .is_some());
        assert!(root.fd_kind(file).is_ok());
        assert!(launch.vfs.fd_kind(3).is_err());
        let bytes = b"a\0b\r\n\xff";
        assert_eq!(
            job.input.as_ref().unwrap().write(bytes).unwrap(),
            bytes.len()
        );
        assert_eq!(launch.vfs.read(0, bytes.len()).unwrap(), bytes);
        launch.vfs.write(1, b"output").unwrap();
        launch.vfs.write(2, b"error").unwrap();
        assert_eq!(job.output[0].as_ref().unwrap().read(16).unwrap(), b"output");
        assert_eq!(job.output[1].as_ref().unwrap().read(16).unwrap(), b"error");
        job.input = None;
        assert!(launch.vfs.read(0, 1).unwrap().is_empty());
        assert!(contender
            .record_lock_conflict(file, wanted)
            .unwrap()
            .is_some());
    }
}
