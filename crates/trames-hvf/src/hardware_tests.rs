// SPDX-License-Identifier: GPL-3.0-or-later
//! Run the signed test binary with --ignored --test-threads=1 on Apple Silicon.
use super::*;
use std::{sync::mpsc, time::Instant};
use trames_memory::Permissions;
fn memory() -> Memory {
    let mut memory = Memory::default();
    memory.map(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    // ldr x1,[x0]; add x1,x1,#1; str x1,[x0]; svc #0
    let code: [u32; 4] = [0xf9400001, 0x91000421, 0xf9000001, 0xd4000001];
    for (i, word) in code.into_iter().enumerate() {
        memory
            .write_array(0x10000 + i as u64 * 4, word.to_le_bytes())
            .unwrap();
    }
    memory.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    memory.map(0x20000, 4 * PAGE_SIZE, Permissions::RW).unwrap();
    memory.write_array(0x20000, 15u64.to_le_bytes()).unwrap();
    memory
}
fn state() -> Aarch64State {
    let mut state = Aarch64State {
        pc: 0x10000,
        sp: 0x30000,
        ..Default::default()
    };
    state.registers[0] = 0x20000;
    state
}
fn svc(executor: &mut Executor, state: &mut Aarch64State, memory: &mut Memory) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        if executor
            .run(state, memory, Duration::from_millis(2))
            .unwrap()
            .reason
            == ExitReason::Syscall
        {
            break;
        }
    }
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn retained_memory_rebinds_after_the_previous_vm_is_destroyed() {
    let mut memory = memory();
    for expected in 16..19 {
        let mut executor = Executor::new().unwrap();
        svc(&mut executor, &mut state(), &mut memory);
        assert_eq!(
            u64::from_le_bytes(memory.read_array(0x20000).unwrap()),
            expected
        );
        drop(executor);
    }
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn host_borrow_parks_hardware_and_fires_one_readiness_notification() {
    let _owner = Executor::new().unwrap();
    let machine = Machine::shared().unwrap();
    let mut memory = memory();
    let mut registry = Registry::default();
    registry.use_hardware();
    memory.prepare_hardware().unwrap();
    let mut view = memory.parallel_view(&mut registry).unwrap();
    view.enable_hardware();
    // SAFETY: all views use hardware mode, no pins, unpublished/native aliases.
    let (storage, _) = unsafe {
        view.export_hardware_page(0x20000, Access::Write, |bytes| {
            machine
                .allocate(bytes)
                .map_err(|_| MemoryError::BackingIo(5))
        })
    }
    .unwrap();
    storage.begin_host_access();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut executor = Executor::new().unwrap();
        let mut state = state();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline);
            let exit = executor
                .run_parallel(&mut state, &mut view, Duration::from_millis(2))
                .unwrap();
            if let Some(wait) = executor.take_wait() {
                let thread = std::thread::current();
                wait.subscribe(move || thread.unpark());
                tx.send(wait.clone()).unwrap();
                while !wait.ready() {
                    std::thread::park();
                }
            }
            if exit.reason == ExitReason::Syscall {
                break;
            }
        }
    });
    let wait = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!wait.ready());
    // SAFETY: the host gate synchronously excludes every hardware actor. The
    // test's retained storage owns this complete initialized 4K allocation.
    assert_eq!(unsafe { storage.pointer().cast::<u64>().read() }, 15);
    storage.end_host_access();
    worker.join().unwrap();
    assert!(wait.ready());
    memory.commit_parallel().unwrap();
    assert_eq!(u64::from_le_bytes(memory.read_array(0x20000).unwrap()), 16);
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn live_dirty_pages_survive_explicit_page_out_and_hardware_page_in() {
    let mut executor = Executor::new().unwrap();
    let mut memory = memory();
    svc(&mut executor, &mut state(), &mut memory);
    let before = paging_stats();
    assert!(before.mapped_bytes > 0);
    assert!(reclaim(u64::MAX).unwrap() > 0);
    assert_eq!(paging_stats().mapped_bytes, 0);
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(u64::from_le_bytes(memory.read_array(0x20000).unwrap()), 17);
    assert!(paging_stats().page_ins > before.page_ins);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn unrelated_guest_pages_reuse_partial_physical_granules() {
    let machine = Machine::shared().unwrap();
    let mut pages: Vec<_> = (0..8)
        .map(|_| Some(machine.allocate(None).unwrap()))
        .collect();
    let base = pages[0].as_ref().unwrap().physical_address() & !16383;
    let before = machine.stats().mapped_bytes;
    for page in pages.iter_mut().take(3) {
        drop(page.take());
    }
    let replacements: Vec<_> = (0..3).map(|_| machine.allocate(None).unwrap()).collect();
    assert!(replacements
        .iter()
        .all(|p| p.physical_address() & !16383 == base));
    assert_eq!(machine.stats().mapped_bytes, before);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn simultaneous_vm_owners_wait_for_last_owner_teardown() {
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..16 {
                    let machine = Machine::shared().unwrap();
                    std::thread::yield_now();
                    drop(machine);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
