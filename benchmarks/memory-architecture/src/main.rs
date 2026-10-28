// SPDX-License-Identifier: GPL-3.0-or-later
// Apple Silicon metadata probe; no guest bytes or HVF CPU are executed.
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("Apple Silicon macOS required: ru_maxrss is measured in bytes here");
use std::time::Instant;
use trames_memory::{parallel::Registry, Memory, Permissions};

fn rss() -> i64 {
    let mut r = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut r) }, 0);
    r.ru_maxrss
}
fn main() {
    let a: Vec<_> = std::env::args().collect();
    let mode = &a[1];
    assert!(matches!(mode.as_str(), "native" | "reserved" | "dense"));
    let mib: usize = a[2].parse().unwrap();
    assert!((1..=1024).contains(&mib));
    let len = mib << 20;
    let base = 0x1_0000_0000;
    let mut rows = Vec::new();
    let initial = rss();
    if mode == "native" {
        let t = Instant::now();
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED);
        rows.push(("map", t.elapsed().as_secs_f64() * 1000., rss()));
        let t = Instant::now();
        assert_eq!(unsafe { libc::mprotect(ptr, len, libc::PROT_READ) }, 0);
        rows.push(("protect", t.elapsed().as_secs_f64() * 1000., rss()));
        let t = Instant::now();
        assert_eq!(unsafe { libc::munmap(ptr, len) }, 0);
        rows.push(("unmap", t.elapsed().as_secs_f64() * 1000., rss()));
    } else {
        let mut m = Memory::new(1usize << 48);
        let mut r = Registry::default();
        r.use_hardware();
        m.prepare_hardware().unwrap();
        let t = Instant::now();
        if mode == "reserved" {
            m.reserve(base, len as u64, false).unwrap();
        } else {
            m.map(base, len as u64, Permissions::RW).unwrap();
        }
        rows.push(("map", t.elapsed().as_secs_f64() * 1000., rss()));
        assert_eq!(m.resident_bytes(), 0);
        let t = Instant::now();
        let mut v = m.parallel_view(&mut r).unwrap();
        v.enable_hardware();
        drop(v);
        rows.push(("freeze", t.elapsed().as_secs_f64() * 1000., rss()));
        assert_eq!(m.resident_bytes(), 0);
        let t = Instant::now();
        m.commit_parallel().unwrap();
        rows.push(("commit", t.elapsed().as_secs_f64() * 1000., rss()));
        let t = Instant::now();
        if mode != "reserved" {
            m.protect(base, len as u64, Permissions::R).unwrap();
        }
        rows.push(("protect", t.elapsed().as_secs_f64() * 1000., rss()));
        let t = Instant::now();
        drop(m.parallel_view(&mut r).unwrap());
        rows.push(("refreeze", t.elapsed().as_secs_f64() * 1000., rss()));
        assert_eq!(m.resident_bytes(), 0);
        let t = Instant::now();
        m.unmap(base, len as u64).unwrap();
        rows.push(("unmap", t.elapsed().as_secs_f64() * 1000., rss()));
        let t = Instant::now();
        drop(m.parallel_view(&mut r).unwrap());
        rows.push(("empty_freeze", t.elapsed().as_secs_f64() * 1000., rss()));
        assert_eq!(m.resident_bytes(), 0);
    }
    print!(
        r#"{{"mode":"{mode}","mib":{mib},"initial_peak_rss_bytes":{initial},"guest_data_touched":false,"stages":["#
    );
    for (i, (name, ms, peak)) in rows.iter().enumerate() {
        if i != 0 {
            print!(",");
        }
        print!(r#"{{"name":"{name}","ms":{ms},"peak_rss_bytes":{peak}}}"#);
    }
    println!("]}}");
}
