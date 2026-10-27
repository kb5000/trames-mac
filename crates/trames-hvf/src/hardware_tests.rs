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

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn shared_file_pixels_survive_import_eviction_and_vm_replacement() {
    let mut linux = trames_linux::Linux::new(0);
    let fd = linux.vfs.memfd_create(b"display-pixels", 0).unwrap();
    linux.vfs.truncate_fd(fd, 4 * PAGE_SIZE).unwrap();
    let file = linux.vfs.shared_file(fd).unwrap();
    let native = file.map(0, (4 * PAGE_SIZE) as usize).unwrap();
    file.write_at(PAGE_SIZE, &15u64.to_le_bytes()).unwrap();
    file.write_at(2 * PAGE_SIZE, &15u64.to_le_bytes()).unwrap();
    let mut memory = memory();
    memory
        .map_backed(
            0x20000,
            linux
                .vfs
                .mapping_pages(fd, 0, 4 * PAGE_SIZE, true, true)
                .unwrap(),
            true,
            Permissions::RW,
            true,
        )
        .unwrap();
    for expected in [16u64, 18, 20] {
        let mut executor = Executor::new().unwrap();
        let mut cpu = state();
        cpu.registers[0] = 0x21000;
        svc(&mut executor, &mut cpu, &mut memory);
        let mapped = paging_stats().mapped_bytes;
        let mut other = state();
        other.registers[0] = 0x22000;
        svc(&mut executor, &mut other, &mut memory);
        assert_eq!(
            paging_stats().mapped_bytes,
            mapped,
            "neighbor file pages must share their host granule"
        );
        assert_eq!(
            unsafe {
                native
                    .pointer()
                    .add(PAGE_SIZE as usize)
                    .cast::<u64>()
                    .read_unaligned()
            },
            expected
        );
        reclaim(u64::MAX).unwrap();
        cpu.pc = 0x10000;
        svc(&mut executor, &mut cpu, &mut memory);
        other.pc = 0x10000;
        svc(&mut executor, &mut other, &mut memory);
        assert_eq!(
            memory.read_array::<8>(0x21000).unwrap(),
            (expected + 1).to_le_bytes()
        );
        assert_eq!(
            unsafe {
                native
                    .pointer()
                    .add(2 * PAGE_SIZE as usize)
                    .cast::<u64>()
                    .read_unaligned()
            },
            expected + 1
        );
        drop(executor);
    }
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn warm_address_spaces_switch_without_reexport_or_page_walks() {
    let mut executor = Executor::new().unwrap();
    let mut a = memory();
    let mut b = memory();
    b.write_array(0x20000, 1000u64.to_le_bytes()).unwrap();
    svc(&mut executor, &mut state(), &mut a);
    svc(&mut executor, &mut state(), &mut b);
    let warm = executor.translations;
    assert!(executor
        .spaces
        .values()
        .all(|space| space.publications.is_empty() && space.private_writable == 1));
    for _ in 0..64 {
        svc(&mut executor, &mut state(), &mut a);
        svc(&mut executor, &mut state(), &mut b);
    }
    assert_eq!(u64::from_le_bytes(a.read_array(0x20000).unwrap()), 80);
    assert_eq!(u64::from_le_bytes(b.read_array(0x20000).unwrap()), 1065);
    let after = executor.translations;
    assert_eq!(after.exports, warm.exports, "{after:?}");
    assert_eq!(after.inspections, warm.inspections, "{after:?}");
    assert_eq!(after.updates, warm.updates, "{after:?}");
    assert_eq!(after.retired, warm.retired, "{after:?}");
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn retained_roots_observe_code_protection_replacement_and_eviction() {
    let mut executor = Executor::new().unwrap();
    let mut a = memory();
    let mut b = memory();
    svc(&mut executor, &mut state(), &mut a);
    svc(&mut executor, &mut state(), &mut b);
    let old_gpa = executor.spaces[&a.instruction_identity().0].pages[&0x10000].gpa;
    a.protect(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    a.write_array(0x10004, 0x91000c21u32.to_le_bytes()).unwrap(); // add x1,x1,#3
    a.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    svc(&mut executor, &mut state(), &mut a);
    assert_eq!(u64::from_le_bytes(a.read_array(0x20000).unwrap()), 19);
    assert_eq!(
        executor.spaces[&a.instruction_identity().0].pages[&0x10000].gpa,
        old_gpa
    );
    let content = executor.translations;
    assert_eq!(
        content.retired, 0,
        "content-only changes retain translations"
    );
    a.protect(0x20000, PAGE_SIZE, Permissions::R).unwrap();
    let result = executor.run(&mut state(), &mut a, Duration::from_millis(20));
    assert!(matches!(
        result,
        Err(Error::Memory(MemoryError::Permission {
            access: Access::Write,
            ..
        }))
    ));
    assert!(executor.translations.updates > content.updates);
    a.unmap(0x20000, PAGE_SIZE).unwrap();
    a.map(0x20000, PAGE_SIZE, Permissions::RW).unwrap();
    a.write_array(0x20000, 100u64.to_le_bytes()).unwrap();
    svc(&mut executor, &mut state(), &mut a);
    assert_eq!(u64::from_le_bytes(a.read_array(0x20000).unwrap()), 103);
    assert!(executor.translations.retired > content.retired);
    let pages_before = executor.translations.exports;
    reclaim(u64::MAX).unwrap();
    svc(&mut executor, &mut state(), &mut b);
    assert_eq!(
        executor.translations.exports, pages_before,
        "physical page-in keeps stage-one mappings"
    );
    assert_eq!(u64::from_le_bytes(b.read_array(0x20000).unwrap()), 17);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn address_space_cache_is_bounded_and_does_not_retain_physical_ram() {
    let mut executor = Executor::new().unwrap();
    let mut memories: Vec<_> = (0..CACHED_SPACES + 4).map(|_| memory()).collect();
    for expected in 16..19u64 {
        for memory in &mut memories {
            svc(&mut executor, &mut state(), memory);
            assert_eq!(
                u64::from_le_bytes(memory.read_array(0x20000).unwrap()),
                expected
            );
            assert!(executor.spaces.len() <= CACHED_SPACES);
        }
    }
    drop(memories);
    assert_eq!(
        executor.mapped_bytes(),
        0,
        "dead weak aliases are not mapped RAM"
    );
    assert_eq!(
        paging_stats().mapped_bytes,
        0,
        "cached PTE metadata must not own guest RAM"
    );
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn raw_host_reads_can_race_hardware_without_gate_or_torn_aligned_words() {
    let _owner = Executor::new().unwrap();
    let mut memory = memory();
    // mvn x1,x1; str x1,[x0]; b -8. Registers alternate two full 64-bit words.
    memory.protect(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    for (i, word) in [0xaa2103e1u32, 0xf9000001, 0x17fffffe]
        .into_iter()
        .enumerate()
    {
        memory
            .write_array(0x10000 + i as u64 * 4, word.to_le_bytes())
            .unwrap();
    }
    memory.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    memory
        .write_array(0x20000, 0x55aa55aa55aa55aau64.to_le_bytes())
        .unwrap();
    let mut registry = Registry::default();
    registry.use_hardware();
    memory.prepare_hardware().unwrap();
    let mut view = memory.parallel_view(&mut registry).unwrap();
    view.enable_hardware();
    let reader = view.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut executor = Executor::new().unwrap();
        let mut state = state();
        state.registers[1] = 0x55aa55aa55aa55aa;
        tx.send(()).unwrap();
        for _ in 0..100 {
            executor
                .run_parallel(&mut state, &mut view, Duration::from_millis(1))
                .unwrap();
            assert!(
                executor.take_wait().is_none(),
                "raw reads must not park hardware"
            );
        }
    });
    rx.recv().unwrap();
    for _ in 0..10000 {
        let bytes = reader.read_array::<8>(0x20000).unwrap();
        let word = u64::from_le_bytes(bytes);
        assert!(
            word == 0x55aa55aa55aa55aa || word == 0xaa55aa55aa55aa55,
            "torn aligned word: {word:x}"
        );
    }
    drop(reader);
    worker.join().unwrap();
    memory.commit_parallel().unwrap();
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn host_code_writes_publish_only_their_range_without_consumer_rescans() {
    let mut executor = Executor::new().unwrap();
    let mut memory = Memory::default();
    memory
        .map(
            0x10000,
            PAGE_SIZE,
            Permissions {
                read: true,
                write: true,
                execute: true,
            },
        )
        .unwrap();
    memory
        .write_array(0x10000, 0xd28000e0u32.to_le_bytes())
        .unwrap(); // mov x0,#7
    memory
        .write_array(0x10004, 0xd4000001u32.to_le_bytes())
        .unwrap();
    memory.prepare_hardware().unwrap();
    let mut registry = Registry::default();
    registry.use_hardware();
    let mut view = memory.parallel_view(&mut registry).unwrap();
    view.enable_hardware();
    let mut writer = view.clone();
    for expected in [7, 9, 13, 15, 17] {
        writer
            .write_array(
                0x10000,
                (0xd2800000u32 | (expected as u32) << 5).to_le_bytes(),
            )
            .unwrap();
        let mut state = state();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < deadline);
            if executor
                .run_parallel(&mut state, &mut view, Duration::from_millis(2))
                .unwrap()
                .reason
                == ExitReason::Syscall
            {
                break;
            }
        }
        assert_eq!(state.registers[0], expected);
    }
    assert_eq!(
        executor.translations.inspections, 0,
        "raw code writes publish at the producer"
    );
    assert_eq!(executor.translations.exports, 1);
    drop(writer);
    drop(view);
    memory.commit_parallel().unwrap();
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn one_mapping_change_checks_one_cached_page() {
    let mut executor = Executor::new().unwrap();
    let mut memory = memory();
    memory
        .map(0x40000, PAGE_SIZE * 32, Permissions::RW)
        .unwrap();
    for page in 0..32 {
        let mut cpu = state();
        cpu.registers[0] = 0x40000 + page * PAGE_SIZE;
        svc(&mut executor, &mut cpu, &mut memory);
    }
    let before = executor.translations;
    memory.protect(0x4f000, PAGE_SIZE, Permissions::R).unwrap();
    let mut cpu = state();
    cpu.registers[0] = 0x40000;
    svc(&mut executor, &mut cpu, &mut memory);
    let after = executor.translations;
    assert_eq!(
        after.inspections - before.inspections,
        1,
        "single-range delta must not walk the image"
    );
    assert_eq!(after.exports, before.exports);
    assert_eq!(after.retired, before.retired);
    assert_eq!(after.updates - before.updates, 1);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn serial_and_worker_dispatch_keep_one_snapshot_registration_domain() {
    use trames_engine::{Cpu as _, SliceOutcome};
    let mut memory = memory();
    let mut registry = Registry::default();
    let mut serial = crate::kernel::Cpu::default();
    let mut engine = crate::kernel::Engine::default();
    let (send, receive) = mpsc::sync_channel::<Option<trames_memory::parallel::View>>(0);
    let (done, joined) = mpsc::sync_channel(0);
    let worker = std::thread::spawn(move || {
        let mut executor = Executor::new().unwrap();
        while let Some(mut view) = receive.recv().unwrap() {
            let mut task = state();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline);
                if executor
                    .run_parallel(&mut task, &mut view, Duration::from_millis(2))
                    .unwrap()
                    .reason
                    == ExitReason::Syscall
                {
                    break;
                }
            }
            drop(view);
            done.send(()).unwrap();
        }
    });
    registry.use_hardware();
    memory.prepare_hardware().unwrap();
    drop(memory.parallel_view(&mut registry).unwrap());
    assert_eq!(registry.metrics.full_rebuilds, 1);
    for round in 0..8 {
        serial.restore_architectural_state(&state());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline);
            match serial.run_coordinated(&mut engine, &mut memory, &mut registry, 4096) {
                SliceOutcome::Syscall { .. } => break,
                SliceOutcome::Yield { .. } => {}
                outcome => panic!("unexpected serial outcome: {outcome:?}"),
            }
        }
        let mut view = memory.parallel_view(&mut registry).unwrap();
        view.enable_hardware();
        send.send(Some(view)).unwrap();
        joined.recv_timeout(Duration::from_secs(5)).unwrap();
        memory.commit_parallel().unwrap();
        assert_eq!(
            u64::from_le_bytes(memory.read_array(0x20000).unwrap()),
            17 + round * 2
        );
        assert_eq!(
            registry.metrics.full_rebuilds, 1,
            "CPU dispatch changes cannot rebuild the address space"
        );
    }
    send.send(None).unwrap();
    worker.join().unwrap();
}
