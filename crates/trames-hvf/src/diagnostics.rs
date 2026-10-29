// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in creator-thread diagnostics; absent from default builds. No byte-path
//! instrumentation, shared counters, monitor thread, or ordinary-memory fence.
use std::{
    cell::RefCell,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy)]
pub(super) enum Work {
    Sync,
    Run,
    Fault,
    Resume,
    AllocateWait,
    Map,
    Initialize,
    Unmap,
    Recycle,
}
#[derive(Clone, Copy)]
pub(super) enum Count {
    Fallback,
    Delta,
    Inspect,
    Export,
    Fault,
    Retire,
    RootSwitch,
    PeakLeaves,
    ChunkChecks,
    FaultNewStorage,
    FaultRetainedStorage,
    NeighborInstall,
    FaultWrite,
    FaultRead,
    FaultExecute,
    AllocateZero,
    AllocateCopy,
    ImportStorage,
    SmeContext,
    SmeStreaming,
}
const WORKS: usize = 9;
const COUNTS: usize = 20;
pub(super) struct Stamp {
    wall: Instant,
    cpu: Option<u64>,
}
struct Totals {
    enabled: bool,
    name: String,
    wall: [u64; WORKS],
    cpu: [u64; WORKS],
    calls: [u64; WORKS],
    counts: [u64; COUNTS],
    reported: Instant,
}
thread_local! {static TOTALS: RefCell<Totals> = RefCell::new(Totals {
    enabled: std::env::var_os("TRAMES_HVF_PROFILE").is_some(),
    name: std::thread::current().name().unwrap_or("main").to_string(),
    wall: [0;WORKS], cpu: [0;WORKS], calls: [0;WORKS], counts: [0;COUNTS], reported: Instant::now(),
});}
fn cpu() -> u64 {
    // SAFETY: complete local timespec, no retained pointers. This is host thread
    // CPU accounting (including kernel), not retired guest instruction counts.
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return 0;
    }
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}
pub(super) fn start(work: Work) -> Option<Stamp> {
    let enabled = TOTALS
        .try_with(|totals| totals.borrow().enabled)
        .unwrap_or(false);
    enabled.then(|| Stamp {
        wall: Instant::now(),
        cpu: matches!(
            work,
            Work::Sync | Work::Run | Work::Fault | Work::Unmap | Work::Recycle
        )
        .then(cpu),
    })
}
pub(super) fn end(work: Work, stamp: Option<Stamp>) {
    let Some(stamp) = stamp else { return };
    let wall = stamp.wall.elapsed().as_nanos() as u64;
    let used = stamp.cpu.map_or(0, |before| cpu().saturating_sub(before));
    let _ = TOTALS.try_with(|totals| {
        let mut totals = totals.borrow_mut();
        let index = work as usize;
        totals.wall[index] += wall;
        totals.cpu[index] += used;
        totals.calls[index] += 1;
        if totals.reported.elapsed() >= Duration::from_secs(1) {
            totals.report();
            totals.reported = Instant::now();
        }
    });
}
pub(super) fn count(kind: Count, value: usize) {
    let _ = TOTALS.try_with(|totals| {
        let mut totals = totals.borrow_mut();
        if totals.enabled {
            let entry = &mut totals.counts[kind as usize];
            if matches!(kind, Count::PeakLeaves) {
                *entry = (*entry).max(value as u64)
            } else {
                *entry += value as u64
            }
        }
    });
}
impl Totals {
    fn report(&self) {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        eprintln!(
            "trames hvf profile: {{\"thread\":{:?},\"at_ms\":{},\"wall_ns\":{:?},\"cpu_ns\":{:?},\"calls\":{:?},\"counts\":{:?}}}",
            self.name, at, self.wall, self.cpu, self.calls, self.counts
        );
    }
}
impl Drop for Totals {
    fn drop(&mut self) {
        if self.enabled {
            self.report();
        }
    }
}
