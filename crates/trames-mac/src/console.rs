// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent guest PTYs over local sockets, pumped by the existing host hook.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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
    time::Duration,
};
use trames_distro::terminals::{Channel, LIMIT, QUEUE_LIMIT};
const LIST: &[u8; 8] = b"TRMLIST1";
const MAGIC: &[u8; 8] = b"TRAMES02";
struct Peer {
    stream: UnixStream,
    incoming: Vec<u8>,
    outgoing: VecDeque<u8>,
    id: Option<u64>,
    detached: bool,
}
impl Peer {
    fn flush(&mut self) -> bool {
        while !self.outgoing.is_empty() {
            match self.stream.write(self.outgoing.as_slices().0) {
                Ok(0) => return false,
                Ok(n) => {
                    self.outgoing.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }
}
pub struct Console {
    listener: UnixListener,
    path: PathBuf,
    peers: Vec<Peer>,
    pub channels: BTreeMap<u64, Channel>,
    next: u64,
    history: VecDeque<u8>,
    histories: BTreeMap<u64, VecDeque<u8>>,
}
impl Console {
    pub fn bind(path: &Path) -> io::Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(m) if !m.file_type().is_socket() => {
                return Err(io::Error::other("console path is not a socket"))
            }
            Ok(_) => fs::remove_file(path)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.to_owned(),
            peers: Vec::new(),
            channels: BTreeMap::new(),
            next: 1,
            history: VecDeque::new(),
            histories: BTreeMap::new(),
        })
    }
    pub fn poll(&mut self) -> io::Result<()> {
        // No new poll loop/thread: accept/read a bounded amount on the original
        // scheduler's host-input opportunity. Structural opens alone request join.
        match self.listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(true)?;
                if self.peers.len() < LIMIT {
                    self.peers.push(Peer {
                        stream,
                        incoming: Vec::new(),
                        outgoing: VecDeque::new(),
                        id: None,
                        detached: false,
                    });
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e),
        }
        let mut active: BTreeSet<_> = self.peers.iter().filter_map(|p| p.id).collect();
        for mut peer in std::mem::take(&mut self.peers) {
            let mut alive = peer.flush();
            let room = peer
                .id
                .and_then(|id| self.channels.get(&id))
                .map_or(QUEUE_LIMIT, |c| QUEUE_LIMIT - c.input.len());
            if alive && peer.incoming.len() + 4096 <= room {
                let mut bytes = [0; 4096];
                match peer.stream.read(&mut bytes) {
                    Ok(0) => alive = false,
                    Ok(n) => peer.incoming.extend_from_slice(&bytes[..n]),
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => alive = false,
                }
            }
            if alive && peer.id.is_none() && peer.incoming.len() >= 16 {
                if &peer.incoming[..8] == LIST {
                    let rows: Vec<_> = self.channels.iter().map(|(&id,c)| serde_json::json!({"id":id,"state":c.phase,"attached":active.contains(&id)})).collect();
                    let bytes = serde_json::to_vec(&rows)?;
                    peer.outgoing.extend((bytes.len() as u32).to_be_bytes());
                    peer.outgoing.extend(bytes);
                    let _ = peer.flush();
                    continue;
                } else if &peer.incoming[..8] != MAGIC {
                    alive = false;
                } else {
                    let requested = u64::from_be_bytes(peer.incoming[8..16].try_into().unwrap());
                    peer.incoming.drain(..16);
                    let id = if requested == 0 && self.channels.len() < LIMIT {
                        let id = self.next;
                        self.next = self
                            .next
                            .checked_add(1)
                            .ok_or_else(|| io::Error::other("terminal ID exhausted"))?;
                        self.channels.insert(id, Channel::default());
                        id
                    } else {
                        requested
                    };
                    if id == 0 || !self.channels.contains_key(&id) || active.contains(&id) {
                        let mut reply = MAGIC.to_vec();
                        reply.push(1);
                        reply.extend_from_slice(&id.to_be_bytes());
                        let _ = peer.stream.write(&reply);
                        alive = false;
                    } else {
                        active.insert(id);
                        peer.id = Some(id);
                        peer.outgoing.extend(MAGIC);
                        peer.outgoing.push_back(0);
                        peer.outgoing.extend(id.to_be_bytes());
                        if requested == 0 {
                            peer.outgoing.extend(self.history.iter().copied());
                        } else if let Some(history) = self.histories.get(&id) {
                            peer.outgoing.extend(history.iter().copied());
                        }
                    }
                }
            }
            while alive && peer.incoming.len() >= 5 {
                let Some(id) = peer.id else {
                    break;
                };
                let kind = peer.incoming[0];
                let n = u32::from_be_bytes(peer.incoming[1..5].try_into().unwrap()) as usize;
                if n > 4096 || !matches!((kind, n), (0, _) | (1, 4) | (2, 0)) {
                    alive = false;
                    break;
                }
                if peer.incoming.len() < 5 + n {
                    break;
                }
                let channel = self.channels.get_mut(&id).unwrap();
                match kind {
                    0 => channel.input.extend(&peer.incoming[5..5 + n]),
                    1 => {
                        let rows = u16::from_be_bytes(peer.incoming[5..7].try_into().unwrap());
                        let cols = u16::from_be_bytes(peer.incoming[7..9].try_into().unwrap());
                        if rows != 0 && cols != 0 {
                            channel.size = Some((rows, cols));
                        }
                    }
                    2 => {
                        peer.detached = true;
                        alive = false;
                    }
                    _ => unreachable!(),
                }
                peer.incoming.drain(..5 + n);
            }
            if alive {
                alive = self.flush_peer(&mut peer);
            }
            if alive {
                self.peers.push(peer);
            } else if let Some(id) = peer.id {
                active.remove(&id);
                if !peer.detached {
                    self.channels.remove(&id);
                    self.histories.remove(&id);
                }
            }
        }
        Ok(())
    }
    fn flush_peer(&mut self, peer: &mut Peer) -> bool {
        if let Some(channel) = peer.id.and_then(|id| self.channels.get_mut(&id)) {
            let take = (QUEUE_LIMIT.saturating_sub(peer.outgoing.len())).min(channel.output.len());
            let bytes: Vec<_> = channel.output.drain(..take).collect();
            if !bytes.is_empty() {
                let history = self.histories.entry(peer.id.unwrap()).or_default();
                let keep = &bytes[bytes.len().saturating_sub(16384)..];
                let excess = (history.len() + keep.len()).saturating_sub(16384);
                history.drain(..excess);
                history.extend(keep);
                peer.outgoing.extend(bytes);
            }
            if channel.phase == "failed" {
                peer.outgoing.extend(
                    format!(
                        "\r\nTerminal failed: {}\r\n",
                        channel.error.as_deref().unwrap_or("unknown error")
                    )
                    .bytes(),
                );
                channel.phase = "exited";
            }
            if !peer.flush() {
                return false;
            }
            if matches!(channel.phase, "exited" | "failed")
                && channel.output.is_empty()
                && peer.outgoing.is_empty()
            {
                return false;
            }
        }
        true
    }
    pub fn flush(&mut self) {
        for mut peer in std::mem::take(&mut self.peers) {
            if self.flush_peer(&mut peer) {
                self.peers.push(peer);
            } else if let Some(id) = peer.id {
                self.channels.remove(&id);
                self.histories.remove(&id);
            }
        }
    }
    pub fn write(&mut self, bytes: &[u8]) -> usize {
        // Only PID 1's boot console, never other terminals' private output.
        let keep = &bytes[bytes.len().saturating_sub(16384)..];
        let excess = (self.history.len() + keep.len()).saturating_sub(16384);
        self.history.drain(..excess);
        self.history.extend(keep);
        for peer in &mut self.peers {
            if peer.id.is_some() && peer.outgoing.len() + bytes.len() <= QUEUE_LIMIT {
                peer.outgoing.extend(bytes);
                let _ = peer.flush();
            }
        }
        bytes.len()
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn frame(peer: &mut UnixStream, kind: u8, bytes: &[u8]) -> io::Result<()> {
    peer.write_all(&[kind])?;
    peer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    peer.write_all(bytes)
}
pub fn attach(path: &Path, terminal: u64) -> io::Result<()> {
    let mut peer = UnixStream::connect(path)?;
    peer.set_read_timeout(Some(Duration::from_secs(10)))?;
    peer.set_write_timeout(Some(Duration::from_secs(10)))?;
    peer.write_all(MAGIC)?;
    peer.write_all(&terminal.to_be_bytes())?;
    let mut reply = [0; 17];
    peer.read_exact(&mut reply)?;
    if &reply[..8] != MAGIC || reply[8] != 0 {
        return Err(io::Error::other(
            "terminal unavailable, already attached, or terminal limit reached",
        ));
    }
    let id = u64::from_be_bytes(reply[9..].try_into().unwrap());
    peer.set_read_timeout(None)?;
    let _signals = super::lifecycle::Signals::install()?;
    let _raw = super::session::RawTerminal::enter(true)?;
    eprintln!("Attached terminal {id}. Ctrl+] detaches; reconnect with session attach {id}.\r");
    let mut size = None;
    let mut bytes = [0; 4096];
    loop {
        if super::lifecycle::stopping() {
            return Ok(());
        }
        let now = super::session::window_size();
        if now != size {
            size = now;
            if let Some((r, c)) = size {
                let mut b = [0; 4];
                b[..2].copy_from_slice(&r.to_be_bytes());
                b[2..].copy_from_slice(&c.to_be_bytes());
                frame(&mut peer, 1, &b)?;
            }
        }
        let mut fds = [
            libc::pollfd {
                fd: peer.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 250) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if fds[0].revents != 0 {
            let n = peer.read(&mut bytes)?;
            if n == 0 {
                return Ok(());
            }
            io::stdout().write_all(&bytes[..n])?;
            io::stdout().flush()?;
        }
        if fds[1].revents != 0 {
            let n = io::stdin().read(&mut bytes)?;
            if n == 0 {
                return Ok(());
            }
            if let Some(at) = bytes[..n].iter().position(|b| *b == 0x1d) {
                if at != 0 {
                    frame(&mut peer, 0, &bytes[..at])?;
                }
                frame(&mut peer, 2, &[])?;
                return Ok(());
            }
            frame(&mut peer, 0, &bytes[..n])?;
        }
    }
}
pub fn list(path: &Path, args: &[String]) -> io::Result<()> {
    if args.len() > 1
        || args
            .first()
            .is_some_and(|a| !["--json", "-q", "--quiet"].contains(&a.as_str()))
    {
        return Err(io::Error::other("usage: session ls [--json|-q]"));
    }
    let mut peer = UnixStream::connect(path)?;
    peer.set_read_timeout(Some(Duration::from_secs(5)))?;
    peer.write_all(LIST)?;
    peer.write_all(&0u64.to_be_bytes())?;
    let mut size = [0; 4];
    peer.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size > 65536 {
        return Err(io::Error::other(
            "invalid session list response; restart the environment",
        ));
    }
    let mut bytes = vec![0; size];
    peer.read_exact(&mut bytes)?;
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
    match args.first().map(String::as_str).unwrap_or("table") {
        "--json" => println!("{}", serde_json::to_string_pretty(&rows)?),
        "-q" | "--quiet" => {
            for row in rows {
                println!("{}", row["id"]);
            }
        }
        _ => {
            println!("ID      STATE       ATTACHED");
            for row in rows {
                println!(
                    "{:<8}{:<12}{}",
                    row["id"],
                    row["state"].as_str().unwrap_or("unknown"),
                    row["attached"]
                );
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn server() -> Console {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        Console::bind(&std::env::temp_dir().join(format!(
            "trames-console-{}-{}.sock",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )))
        .unwrap()
    }
    fn connect(s: &mut Console, id: u64) -> (UnixStream, u64) {
        let mut p = UnixStream::connect(&s.path).unwrap();
        p.write_all(MAGIC).unwrap();
        p.write_all(&id.to_be_bytes()).unwrap();
        s.poll().unwrap();
        let mut b = [0; 17];
        p.read_exact(&mut b).unwrap();
        assert_eq!(&b[..9], b"TRAMES02\0");
        (p, u64::from_be_bytes(b[9..].try_into().unwrap()))
    }
    #[test]
    fn independent_channels_keep_input_size_and_output_separate() {
        let mut s = server();
        let (mut a, aid) = connect(&mut s, 0);
        let (mut b, bid) = connect(&mut s, 0);
        assert_ne!(aid, bid);
        frame(&mut a, 0, b"first").unwrap();
        frame(&mut b, 1, &[0, 40, 0, 120]).unwrap();
        s.poll().unwrap();
        assert_eq!(
            s.channels[&aid].input.iter().copied().collect::<Vec<_>>(),
            b"first"
        );
        assert!(s.channels[&bid].input.is_empty());
        assert_eq!(s.channels[&bid].size, Some((40, 120)));
        s.channels.get_mut(&aid).unwrap().output.extend(b"private");
        s.flush();
        let mut bytes = [0; 7];
        a.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"private");
        b.set_nonblocking(true).unwrap();
        assert_eq!(
            b.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn detach_retains_session_but_closed_window_releases_only_its_channel() {
        let mut s = server();
        let (mut a, id) = connect(&mut s, 0);
        s.channels.get_mut(&id).unwrap().output.extend(b"prompt # ");
        s.flush();
        let mut replay = [0; 9];
        a.read_exact(&mut replay).unwrap();
        assert_eq!(&replay, b"prompt # ");
        frame(&mut a, 2, &[]).unwrap();
        s.poll().unwrap();
        drop(a);
        assert!(s.channels.contains_key(&id));
        let (mut a, same) = connect(&mut s, id);
        a.read_exact(&mut replay).unwrap();
        assert_eq!(&replay, b"prompt # ");
        assert_eq!(id, same);
        let (_b, other) = connect(&mut s, 0);
        drop(a);
        s.poll().unwrap();
        assert!(!s.channels.contains_key(&id));
        assert!(s.channels.contains_key(&other));
    }
    #[test]
    fn fragmented_and_oversize_frames_do_not_cross_terminal_boundaries() {
        let mut s = server();
        let (mut p, id) = connect(&mut s, 0);
        p.write_all(&[0, 0, 0]).unwrap();
        s.poll().unwrap();
        assert!(s.channels[&id].input.is_empty());
        p.write_all(&[0, 2, b'o', b'k']).unwrap();
        s.poll().unwrap();
        assert_eq!(s.channels[&id].input.len(), 2);
        p.write_all(&[0, 0, 1, 0, 1]).unwrap();
        s.poll().unwrap();
        assert!(!s.channels.contains_key(&id));
    }
}
