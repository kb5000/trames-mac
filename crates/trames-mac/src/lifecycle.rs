// SPDX-License-Identifier: GPL-3.0-or-later
//! Host cancellation uses native signal/control readiness and source completion.
use std::{
    io,
    os::fd::AsRawFd,
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
};
static STOP: AtomicBool = AtomicBool::new(false);
static SIGNAL: AtomicI32 = AtomicI32::new(0);
static CONTROL: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_PIPE: std::sync::OnceLock<(
    std::os::unix::net::UnixStream,
    std::os::unix::net::UnixStream,
)> = std::sync::OnceLock::new();
pub fn interest(wait: &mut trames_linux::HostWait) -> io::Result<()> {
    if let Some((read, _)) = SIGNAL_PIPE.get() {
        wait.watch(read, false)?;
    }
    let fd = CONTROL.load(Ordering::Relaxed);
    if fd >= 0 {
        // SessionControl outlives Runtime::run and the synchronous duplication.
        wait.watch(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }, false)?;
    }
    Ok(())
}
extern "C" fn cancel(signal: libc::c_int) {
    SIGNAL.store(signal, Ordering::Relaxed);
    STOP.store(true, Ordering::Relaxed);
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        unsafe {
            libc::write(fd, b"s".as_ptr().cast(), 1);
        }
    }
}
pub fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}
pub fn take_signal() -> Option<i32> {
    let signal = SIGNAL.swap(0, Ordering::Relaxed);
    STOP.store(false, Ordering::Relaxed);
    (signal != 0).then_some(signal)
}
pub fn exit_code() -> u8 {
    let signal = SIGNAL.load(Ordering::Relaxed);
    if signal == 0 {
        0
    } else {
        (128 + signal) as u8
    }
}
pub fn ready() {
    eprintln!("trames: session ready");
}
pub fn poll_stop() {
    if let Some((read, _)) = SIGNAL_PIPE.get() {
        use std::io::Read;
        let mut byte = [0; 256];
        while (&*read).read(&mut byte).is_ok_and(|n| n != 0) {}
    }
    let fd = CONTROL.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    // SAFETY: SessionControl owns this nonblocking listener until all execution
    // threads have joined. A connection is an explicit local stop request.
    let peer = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    if peer >= 0 {
        unsafe {
            libc::close(peer);
        }
        STOP.store(true, Ordering::Relaxed);
    }
}
pub struct SessionControl(std::os::unix::net::UnixListener, std::path::PathBuf);
impl SessionControl {
    pub fn bind(path: &std::path::Path) -> io::Result<Self> {
        let _ = std::fs::remove_file(path);
        let socket = std::os::unix::net::UnixListener::bind(path)?;
        socket.set_nonblocking(true)?;
        CONTROL.store(socket.as_raw_fd(), Ordering::Relaxed);
        Ok(Self(socket, path.to_owned()))
    }
}
impl Drop for SessionControl {
    fn drop(&mut self) {
        CONTROL.store(-1, Ordering::Relaxed);
        let _ = self.0.as_raw_fd();
        let _ = std::fs::remove_file(&self.1);
    }
}
pub struct Signals(Vec<(i32, libc::sigaction)>);
impl Signals {
    pub fn install() -> io::Result<Self> {
        if SIGNAL_PIPE.get().is_none() {
            let (read, write) = std::os::unix::net::UnixStream::pair()?;
            read.set_nonblocking(true)?;
            write.set_nonblocking(true)?;
            let _ = SIGNAL_PIPE.set((read, write));
        }
        SIGNAL_FD.store(SIGNAL_PIPE.get().unwrap().1.as_raw_fd(), Ordering::Relaxed);
        STOP.store(false, Ordering::Relaxed);
        let mut saved = Self(Vec::new());
        for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
            // SAFETY: initialized platform sigaction; handler only writes one
            // lock-free atomic. SIGPIPE becomes an ordinary EPIPE write error.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                let mut old = std::mem::zeroed();
                action.sa_sigaction = if signal == libc::SIGPIPE {
                    libc::SIG_IGN
                } else {
                    cancel as *const () as usize
                };
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, &mut old) != 0 {
                    return Err(io::Error::last_os_error());
                }
                saved.0.push((signal, old));
            }
        }
        Ok(saved)
    }
}
impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, action) in &self.0 {
            unsafe {
                libc::sigaction(*signal, action, std::ptr::null_mut());
            }
        }
    }
}
