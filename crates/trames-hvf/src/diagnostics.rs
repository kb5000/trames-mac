// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in creator-thread diagnostics; absent from default builds. No byte-path
//! instrumentation, shared counters, monitor thread, or ordinary-memory fence.
use std::{
    cell::RefCell,
    collections::BTreeMap,
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
    WorkerMigration,
    OwnerTransition,
}
#[repr(C)]
struct TimebaseInfo {
    numer: u32,
    denom: u32,
}
unsafe extern "C" {
    fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
    fn hv_vcpu_get_exec_time(cpu: u64, ticks: *mut u64) -> i32;
}
fn execution_ticks(cpu: u64) -> Option<u64> {
    let mut ticks = 0;
    // SAFETY: called on the registered vCPU's owning thread, before destruction;
    // the SDK writes one local u64 in mach_absolute_time units.
    (unsafe { hv_vcpu_get_exec_time(cpu, &mut ticks) } == 0).then_some(ticks)
}
fn execution_ns(ticks: u64) -> u64 {
    static RATIO: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let &(numer, denom) = RATIO.get_or_init(|| {
        let mut info = TimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: local, complete timebase output; no retained pointers.
        let code = unsafe { mach_timebase_info(&mut info) };
        assert!(code == 0 && info.numer != 0 && info.denom != 0);
        (info.numer, info.denom)
    });
    ((ticks as u128 * numer as u128) / denom as u128).min(u64::MAX as u128) as u64
}
struct LiveCpu {
    host: *mut std::ffi::c_void,
    ticks: u64,
    registers: [u64; 11],
}
pub(super) fn cpu_created(cpu: u64, host: *mut std::ffi::c_void) {
    let _ = TOTALS.try_with(|totals| {
        let mut totals = totals.borrow_mut();
        if totals.enabled {
            totals.vcpus.insert(
                cpu,
                LiveCpu {
                    host,
                    ticks: 0,
                    registers: [0; 11],
                },
            );
        }
    });
}
pub(super) fn cpu_destroying(cpu: u64) {
    let _ = TOTALS.try_with(|totals| {
        let mut totals = totals.borrow_mut();
        if !totals.enabled {
            return;
        }
        totals.sample_execution();
        if let Some(live) = totals.vcpus.remove(&cpu) {
            totals.retired_execution_ticks += live.ticks;
            for (total, value) in totals.retired_registers.iter_mut().zip(live.registers) {
                *total += value;
            }
        }
        totals.report();
    });
}
const WORKS: usize = 9;
const COUNTS: usize = 22;
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
    vcpus: BTreeMap<u64, LiveCpu>,
    retired_registers: [u64; 11],
    syscalls: [u64; 512],
    protections: [u64; 8],
    retired_execution_ticks: u64,
    execution_errors: u64,
}
thread_local! {static TOTALS: RefCell<Totals> = RefCell::new(Totals {
    enabled: std::env::var_os("TRAMES_HVF_PROFILE").is_some(),
    name: std::thread::current().name().unwrap_or("main").to_string(),
    wall: [0;WORKS], cpu: [0;WORKS], calls: [0;WORKS], counts: [0;COUNTS], reported: Instant::now(),
    vcpus: BTreeMap::new(), retired_execution_ticks: 0, execution_errors: 0,
    retired_registers: [0;11], syscalls: [0;512], protections: [0;8],
});}
pub(crate) fn binding(previous: &mut Option<(std::thread::ThreadId, bool)>) {
    let enabled = TOTALS
        .try_with(|totals| totals.borrow().enabled)
        .unwrap_or(false);
    if !enabled {
        return;
    }
    let thread = std::thread::current();
    let here = (
        thread.id(),
        thread
            .name()
            .is_some_and(|name| name.starts_with("trames-cpu-")),
    );
    if let Some(old) = previous.replace(here) {
        if old.0 != here.0 {
            count(
                if old.1 && here.1 {
                    Count::WorkerMigration
                } else {
                    Count::OwnerTransition
                },
                1,
            );
        }
    }
}
pub(super) fn syscall(number: u32, argument2: u64) {
    let _ = TOTALS.try_with(|totals| {
        let mut totals = totals.borrow_mut();
        if !totals.enabled {
            return;
        }
        if let Some(counter) = totals.syscalls.get_mut(number as usize) {
            *counter += 1;
        }
        if number == 226 {
            totals.protections[(argument2 & 7) as usize] += 1;
        }
    });
}
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
    fn sample_execution(&mut self) {
        for (&cpu, live) in &mut self.vcpus {
            match execution_ticks(cpu) {
                Some(now) => live.ticks = now,
                None => self.execution_errors += 1,
            }
            let mut r = super::RegisterStats::default();
            // SAFETY: registration and removal bracket the owning Executor's
            // lifetime. This TLS report runs on its creator with the CPU stopped.
            unsafe { super::trames_hvf_register_stats(live.host, &mut r) };
            live.registers = [
                r.mailbox,
                r.fallback,
                r.completions,
                r.entries,
                r.invalidations,
                r.full_invalidations,
                r.restore_interruptions,
                r.fault_continuations,
                r.timer_programs,
                r.publications,
                r.cancellations,
            ];
        }
    }
    fn report(&mut self) {
        self.sample_execution();
        let execution = execution_ns(
            self.retired_execution_ticks + self.vcpus.values().map(|live| live.ticks).sum::<u64>(),
        );
        let mut registers = self.retired_registers;
        for live in self.vcpus.values() {
            for (sum, value) in registers.iter_mut().zip(live.registers) {
                *sum += value;
            }
        }
        let syscalls: Vec<_> = self
            .syscalls
            .iter()
            .enumerate()
            .filter(|(_, n)| **n != 0)
            .map(|(i, n)| [i as u64, *n])
            .collect();
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        eprintln!(
            "trames hvf profile: {{\"thread\":{:?},\"at_ms\":{},\"wall_ns\":{:?},\"cpu_ns\":{:?},\"calls\":{:?},\"counts\":{:?},\"vcpu_exec_ns\":{},\"vcpu_exec_errors\":{},\"registers\":{:?},\"svc\":{:?},\"mprotect\":{:?}}}",
            self.name, at, self.wall, self.cpu, self.calls, self.counts, execution, self.execution_errors, registers, syscalls, self.protections
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
