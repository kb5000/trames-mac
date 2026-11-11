// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn pressure_batches_are_bounded_and_do_not_become_a_residency_cap() {
    let machine = Machine::shared().unwrap();
    let slots: Vec<_> = (0..4 * 130)
        .map(|_| machine.allocate(None).unwrap())
        .collect();
    let before = machine.stats();
    let mut request = reclaim_request(u64::MAX).unwrap();
    assert!(!request.progress().unwrap());
    let first = machine.stats();
    assert!(first.mapped_bytes < before.mapped_bytes);
    assert_eq!(
        before.mapped_bytes - first.mapped_bytes,
        64 * machine.granule as u64
    );
    assert!(!request.progress().unwrap());
    assert!(request.progress().unwrap());
    assert_eq!(machine.stats().mapped_bytes, 0);
    for slot in &slots {
        assert!(machine.resume(slot.physical_address()).unwrap().is_none());
    }
    let restored = machine.stats();
    assert_eq!(restored.mapped_bytes, before.mapped_bytes);
    assert!(request.progress().unwrap());
    assert_eq!(machine.stats().page_out_bytes, restored.page_out_bytes);
    assert_eq!(machine.stats().mapped_bytes, restored.mapped_bytes);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn pressure_waits_on_original_byte_lease_and_preserves_host_writes() {
    let machine = Machine::shared().unwrap();
    let slot = machine.allocate(None).unwrap();
    let block = machine.physical(slot.physical_address()).unwrap();
    slot.begin_host_access();
    // SAFETY: this original slot owns the initialized range and host byte lease.
    unsafe { slot.pointer().cast::<u64>().write(41) };
    let before = machine.stats();
    assert_eq!(reclaim(u64::MAX).unwrap(), 0);
    assert!(block.gate.lock().unwrap().waiters.is_empty());
    let mut request = reclaim_request(u64::MAX).unwrap();
    assert!(!request.progress().unwrap());
    assert!(matches!(request.phase, ReclaimPhase::Waiting));
    assert_eq!(block.gate.lock().unwrap().waiters.len(), 1);
    for _ in 0..8 {
        assert!(!request.progress().unwrap());
    }
    assert_eq!(
        block.gate.lock().unwrap().waiters.len(),
        1,
        "no polling/resubscription"
    );
    assert_eq!(machine.stats().mapped_bytes, before.mapped_bytes);
    unsafe { slot.pointer().cast::<u64>().write(99) };
    slot.end_host_access();
    assert!(request.progress().unwrap());
    assert_eq!(machine.stats().mapped_bytes, 0);
    assert!(machine.resume(slot.physical_address()).unwrap().is_none());
    slot.begin_host_access();
    assert_eq!(unsafe { slot.pointer().cast::<u64>().read() }, 99);
    slot.end_host_access();
    // A cancelled request leaves no machine/resource ownership cycle. Its
    // retained readiness can safely expire at the original lease release.
    slot.begin_host_access();
    let mut cancelled = reclaim_request(u64::MAX).unwrap();
    assert!(!cancelled.progress().unwrap());
    drop(cancelled);
    slot.end_host_access();
    assert!(block.gate.lock().unwrap().waiters.is_empty());
    let weak = Arc::downgrade(&machine);
    drop(request);
    drop(slot);
    drop(block);
    drop(machine);
    assert!(weak.upgrade().is_none(), "no retained VM ownership cycle");
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn concurrent_native_growth_retirement_and_reclaim_keep_original_accounting() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Barrier,
    };
    let machine = Machine::shared().unwrap();
    let start = Arc::new(Barrier::new(5));
    let done = Arc::new(AtomicBool::new(false));
    let workers: Vec<_> = (0..4)
        .map(|id| {
            let start = start.clone();
            let machine = machine.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                start.wait();
                for sequence in 0..128 {
                    let storage = machine.allocate_range().unwrap();
                    let expected = 1000 * id + sequence + 1;
                    storage.begin_host_access();
                    // SAFETY: the original host lease owns these initialized bytes.
                    unsafe { storage.pointer().cast::<u64>().write(expected) };
                    storage.end_host_access();
                    assert!(machine
                        .resume(storage.physical_address())
                        .unwrap()
                        .is_none());
                    storage.begin_host_access();
                    assert_eq!(unsafe { storage.pointer().cast::<u64>().read() }, expected);
                    storage.end_host_access();
                }
                done.store(true, Ordering::Relaxed);
            })
        })
        .collect();
    start.wait();
    let mut calls = 0;
    while !done.load(Ordering::Relaxed) || calls < 32 {
        reclaim(u64::MAX).unwrap();
        calls += 1;
        std::thread::park_timeout(std::time::Duration::from_micros(50));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    reclaim(u64::MAX).unwrap();
    let stats = machine.stats();
    assert_eq!(stats.mapped_bytes, 0);
    assert_eq!(stats.backing_bytes, 0);
    assert_eq!(stats.native_range_bytes, 0);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn pressure_native_ranges_respect_the_byte_batch_without_releasing_address_ownership() {
    let machine = Machine::shared().unwrap();
    let ranges: Vec<_> = (0..16).map(|_| machine.allocate_range().unwrap()).collect();
    ranges[0].begin_host_access();
    // SAFETY: original initialized native range, retained by its host byte lease.
    unsafe { ranges[0].pointer().cast::<u64>().write(42) };
    ranges[0].end_host_access();
    let before = machine.stats();
    let mut request = reclaim_request(u64::MAX).unwrap();
    assert!(!request.progress().unwrap());
    assert_eq!(before.mapped_bytes - machine.stats().mapped_bytes, 8 << 20);
    assert!(!request.progress().unwrap());
    assert!(!request.progress().unwrap());
    assert!(request.progress().unwrap());
    assert_eq!(machine.stats().mapped_bytes, 0);
    assert_eq!(
        machine.stats().native_range_bytes,
        before.native_range_bytes
    );
    ranges[0].begin_host_access();
    assert_eq!(unsafe { ranges[0].pointer().cast::<u64>().read() }, 42);
    ranges[0].end_host_access();
    assert!(machine
        .resume(ranges[0].physical_address())
        .unwrap()
        .is_none());
    assert_eq!(machine.stats().mapped_bytes, 2 << 20);
}

#[test]
fn pressure_event_wakes_the_embedding_without_periodic_polling() {
    unsafe extern "C" {
        fn trames_hvf_inject_pressure_for_test(bits: u32);
    }
    let _ = take_memory_pressure().unwrap();
    let mut wait = trames_shared::HostWait::new().unwrap();
    wait.begin();
    pressure_interest(&mut wait).unwrap();
    wait.until(std::time::Instant::now() + std::time::Duration::from_secs(2));
    // Test-only injection follows the real callback's latch and native pipe.
    unsafe {
        trames_hvf_inject_pressure_for_test(4);
    }
    wait.wait().unwrap();
    assert!(
        wait.signal().ready(),
        "pressure did not notify the native waiter"
    );
    assert_eq!(take_memory_pressure().unwrap(), 4);
}

fn imported_test_mapping(length: usize) -> SharedMapping {
    struct Mapping(usize, usize);
    impl Drop for Mapping {
        fn drop(&mut self) {
            assert_eq!(unsafe { libc::munmap(self.0 as *mut c_void, self.1) }, 0);
        }
    }
    let pointer = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_ANON | libc::MAP_PRIVATE,
            -1,
            0,
        )
    };
    assert_ne!(pointer, libc::MAP_FAILED);
    // SAFETY: mmap provides initialized demand-zero complete host granules;
    // the retained owner unmaps only after every imported alias has returned.
    unsafe {
        SharedMapping::from_allocation(pointer.cast(), length, Mapping(pointer as usize, length))
    }
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn imported_granules_retire_without_scanning_other_live_imports() {
    let machine = Machine::shared().unwrap();
    let mapping = imported_test_mapping(machine.granule * 256);
    let slots: Vec<_> = (0..256)
        .map(|i| {
            machine
                .import(mapping.with_offset(i * machine.granule).unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(machine.imports.lock().unwrap().len(), 256);
    let alias = machine
        .import(mapping.with_offset(PAGE_SIZE as usize).unwrap())
        .unwrap();
    assert_eq!(
        alias.physical_address(),
        slots[0].physical_address() + PAGE_SIZE
    );
    drop(slots);
    assert_eq!(machine.imports.lock().unwrap().len(), 1);
    assert_eq!(machine.stats().mapped_bytes, machine.granule as u64);
    drop(alias);
    assert!(machine.imports.lock().unwrap().is_empty());
    assert_eq!(machine.stats().mapped_bytes, 0);
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn concurrent_import_retirement_keeps_replacement_weak_entries() {
    let machine = Machine::shared().unwrap();
    let mapping = imported_test_mapping(machine.granule);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let machine = machine.clone();
            let mapping = mapping.clone();
            scope.spawn(move || {
                for _ in 0..512 {
                    let slot = machine.import(mapping.clone()).unwrap();
                    std::thread::yield_now();
                    drop(slot);
                }
            });
        }
    });
    assert!(machine.imports.lock().unwrap().is_empty());
    assert_eq!(machine.stats().mapped_bytes, 0);
}
