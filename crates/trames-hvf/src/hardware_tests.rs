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
fn concurrent_slot_allocation_preserves_live_bytes_and_releases_idle_ram() {
    const THREADS: usize = 12;
    const ROUNDS: usize = 8;
    const BATCH: usize = 64;
    let machine = Machine::shared().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(THREADS + 1));
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for worker in 0..THREADS {
            let (machine, barrier, tx) = (machine.clone(), barrier.clone(), tx.clone());
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    barrier.wait();
                    let pages: Vec<_> = (0..BATCH)
                        .map(|index| {
                            let value =
                                (worker * ROUNDS * BATCH + round * BATCH + index + 1) as u64;
                            let page = machine.allocate(None).unwrap();
                            // SAFETY: only this test worker owns this unpublished
                            // slot. No byte references or hardware actors exist.
                            for offset in (0..PAGE_SIZE as usize).step_by(8) {
                                unsafe { page.pointer().add(offset).cast::<u64>().write(value) };
                            }
                            (page, value)
                        })
                        .collect();
                    tx.send(pages).unwrap();
                    barrier.wait();
                }
                // Keep the TLS hint alive until the final zero-residency check.
                barrier.wait();
            });
        }
        let mut retained = Vec::new();
        for _ in 0..ROUNDS {
            barrier.wait();
            let mut pages = std::mem::take(&mut retained);
            for _ in 0..THREADS {
                pages.extend(rx.recv_timeout(Duration::from_secs(5)).unwrap());
            }
            let mut addresses = std::collections::BTreeSet::new();
            for (page, value) in &pages {
                assert!(
                    addresses.insert(page.physical_address()),
                    "overlapping live slots"
                );
                for offset in (0..PAGE_SIZE as usize).step_by(8) {
                    // SAFETY: the channel transfers ownership and publishes the
                    // worker writes; every worker waits at the barrier.
                    assert_eq!(
                        unsafe { page.pointer().add(offset).cast::<u64>().read() },
                        *value
                    );
                }
            }
            // Leave holes among live peers, so both partial-granule reuse and
            // whole-granule retirement race with the next allocation batch.
            retained = pages
                .into_iter()
                .enumerate()
                .filter_map(|(i, p)| (i % 17 == 0).then_some(p))
                .collect();
            barrier.wait();
        }
        drop(retained);
        assert_eq!(machine.stats().mapped_bytes, 0);
        assert_eq!(machine.stats().backing_bytes, 0);
        barrier.wait();
    });
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

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn private_writable_code_reuses_metadata_across_joined_epochs() {
    let mut executor = Executor::new().unwrap();
    let mut memory = memory();
    memory
        .protect(
            0x10000,
            PAGE_SIZE,
            Permissions {
                read: true,
                write: true,
                execute: true,
            },
        )
        .unwrap();
    svc(&mut executor, &mut state(), &mut memory);
    let warm = executor.translations;
    let original = memory.hardware_page_info(0x10000).unwrap();
    for _ in 0..64 {
        svc(&mut executor, &mut state(), &mut memory);
        assert_eq!(memory.hardware_page_info(0x10000).unwrap(), original);
    }
    assert!(executor
        .spaces
        .values()
        .all(|space| space.publications.is_empty()));
    assert_eq!(executor.translations.inspections, warm.inspections);
    assert_eq!(executor.translations.exports, warm.exports);
    assert_eq!(executor.translations.updates, warm.updates);
    assert_eq!(executor.translations.retired, warm.retired);
    assert_eq!(u64::from_le_bytes(memory.read_array(0x20000).unwrap()), 80);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn guest_code_stores_use_guest_cache_maintenance_across_cpus() {
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
    // isb; mov x0,#7; svc #0. The receiving CPU supplies its own ISB.
    for (i, word) in [0xd5033fdfu32, 0xd28000e0, 0xd4000001]
        .into_iter()
        .enumerate()
    {
        memory
            .write_array(0x10000 + i as u64 * 4, word.to_le_bytes())
            .unwrap();
    }
    memory.map(0x30000, PAGE_SIZE, Permissions::RW).unwrap();
    // str w1,[x0]; dc cvau,x0; dsb ish; ic ivau,x0; dsb ish; isb; svc #0.
    // Encodings assembled from the architectural instructions with clang.
    for (i, word) in [
        0xb9000001u32,
        0xd50b7b20,
        0xd5033b9f,
        0xd50b7520,
        0xd5033b9f,
        0xd5033fdf,
        0xd4000001,
    ]
    .into_iter()
    .enumerate()
    {
        memory
            .write_array(0x30000 + i as u64 * 4, word.to_le_bytes())
            .unwrap();
    }
    memory.protect(0x30000, PAGE_SIZE, Permissions::RX).unwrap();
    memory.prepare_hardware().unwrap();
    let mut registry = Registry::default();
    registry.use_hardware();
    let mut reader = memory.parallel_view(&mut registry).unwrap();
    reader.enable_hardware();
    let writer = reader.clone();
    let mut executor = Executor::new().unwrap();
    let run = |executor: &mut Executor, view: &mut View, cpu: &mut Aarch64State| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline);
            if executor
                .run_parallel(cpu, view, Duration::from_millis(2))
                .unwrap()
                .reason
                == ExitReason::Syscall
            {
                break;
            }
        }
    };
    let mut cpu = state();
    run(&mut executor, &mut reader, &mut cpu);
    assert_eq!(cpu.registers[0], 7);
    let revision = reader
        .hardware_page_info(0x10000)
        .unwrap()
        .instruction_revision;
    let (request, requests) = mpsc::channel();
    let (complete, completions) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut writer = writer;
        let mut executor = Executor::new().unwrap();
        for expected in requests {
            let mut cpu = state();
            cpu.pc = 0x30000;
            cpu.registers[0] = 0x10004;
            cpu.registers[1] = (0xd2800000u32 | (expected as u32) << 5) as u64;
            run(&mut executor, &mut writer, &mut cpu);
            complete.send(()).unwrap();
        }
    });
    for expected in [9u64, 13, 15, 17] {
        request.send(expected).unwrap();
        completions.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut cpu = state();
        run(&mut executor, &mut reader, &mut cpu);
        assert_eq!(cpu.registers[0], expected);
        assert_eq!(
            reader
                .hardware_page_info(0x10000)
                .unwrap()
                .instruction_revision,
            revision
        );
        assert!(executor
            .spaces
            .values()
            .all(|space| space.publications.is_empty()));
    }
    drop(request);
    worker.join().unwrap();
    drop(reader);
    memory.commit_parallel().unwrap();
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn syscall_arguments_cannot_be_consumed_by_platform_hypercalls() {
    let mut executor = Executor::new().unwrap();
    let mut memory = Memory::default();
    memory.map(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    // svc #0; brk #0x123. Reaching BRK means the syscall exit was swallowed.
    memory
        .write_array(0x10000, 0xd4000001u32.to_le_bytes())
        .unwrap();
    memory
        .write_array(0x10004, 0xd4202460u32.to_le_bytes())
        .unwrap();
    memory.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    for value in [
        0xc0fff000,
        0xc1000000,
        0xc1000001,
        0xc1001000,
        0xc1fff000,
        0xc1ffffff,
        0xc2000000,
        0x84000000,
        0xc4000000,
        0xffffffffc1000000,
        u64::MAX,
    ] {
        let mut cpu = state();
        for (i, register) in cpu.registers.iter_mut().enumerate() {
            *register = 0xa000000000000000 | i as u64;
        }
        cpu.registers[0] = value;
        cpu.registers[8] = 215;
        let expected = cpu.registers;
        svc(&mut executor, &mut cpu, &mut memory);
        assert_eq!(cpu.pc, 0x10004);
        assert_eq!(cpu.registers, expected, "syscall argument {value:#x}");
    }
}

fn context_memory(word: u32) -> Memory {
    let mut m = Memory::default();
    m.map(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    m.write_array(0x10000, word.to_le_bytes()).unwrap();
    m.write_array(0x10004, 0xd4000001u32.to_le_bytes()).unwrap();
    m.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    m
}
fn patterned_state(tag: u64) -> Aarch64State {
    let mut s = state();
    for (i, x) in s.registers.iter_mut().enumerate() {
        *x = 0x8123456789abcdefu64.wrapping_add(tag * 31 + i as u64);
    }
    for (i, q) in s.vectors.iter_mut().enumerate() {
        *q = (u128::from(tag) << 96) | (u128::from(i as u64) << 64) | 0xfedcba9876543210;
    }
    s.sp += tag * 16;
    s.tpidr_el0 = 0xfeed0000 + tag;
    s.fpcr = 0x01400000;
    s.fpsr = 0x0800001f;
    s.nzcv = ((tag as u32) & 15) << 28;
    s
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn context_mailbox_preserves_complete_state_across_tasks_and_roots() {
    let mut e = Executor::new().unwrap();
    let mut a = context_memory(0xd4000001);
    let mut b = context_memory(0xd4000001);
    for tag in 0..128 {
        let mut cpu = patterned_state(tag);
        let mut expected = cpu.clone();
        expected.pc += 4;
        svc(&mut e, &mut cpu, if tag & 1 == 0 { &mut a } else { &mut b });
        assert_eq!(cpu, expected, "context tag {tag}");
    }
    assert!(e.register_stats().mailbox >= 128);
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn interrupted_context_save_and_restore_keep_the_complete_continuation() {
    unsafe extern "C" {
        fn trames_hvf_test_interrupted_context(
            host: *mut c_void,
            state: *mut Aarch64State,
            out: *mut HostExit,
            phase: u32,
        ) -> i32;
    }
    let mut e = Executor::new().unwrap();
    let mut memory = context_memory(0xd4000001);
    svc(&mut e, &mut state(), &mut memory); // install tables and finish TLBI
    for phase in 0..9 {
        let mut cpu = patterned_state(u64::from(phase));
        cpu.pc += 4;
        let expected = cpu.clone();
        let mut out = HostExit::default();
        // SAFETY: same creator-thread executor, retained EL1 context and exact
        // state layout. The hook invokes real adapter instructions and the
        // production completion path, never mutating an HVF-owned exit record.
        check(unsafe {
            trames_hvf_test_interrupted_context(e.host.as_ptr(), &mut cpu, &mut out, phase)
        })
        .unwrap();
        assert_eq!(cpu, expected, "phase {phase}");
        assert_eq!(
            out.kind,
            if (3..=4).contains(&phase) || phase >= 6 {
                0
            } else {
                1
            }
        );
        // Partial loads/saves must not leak into the next task's bank.
        cpu.pc = 0x10000;
        svc(&mut e, &mut cpu, &mut memory);
        assert_eq!(cpu, expected);
    }
    assert!(e.register_stats().completions >= 3);
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn el0_timer_uses_full_register_fallback_without_changing_state() {
    let mut e = Executor::new().unwrap();
    let mut memory = context_memory(0x14000000); // b .
    let mut cpu = patterned_state(9);
    let expected = cpu.clone();
    for _ in 0..3 {
        let exit = e
            .run(&mut cpu, &mut memory, Duration::from_millis(2))
            .unwrap();
        assert_eq!(exit.reason, ExitReason::Yield);
        assert_eq!(cpu, expected);
        if e.register_stats().fallback != 0 {
            break;
        }
    }
    assert!(e.register_stats().fallback != 0);
}
#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn el0_cannot_read_write_or_execute_the_private_context() {
    let mut e = Executor::new().unwrap();
    for word in [0xf9400020, 0xf9000020, 0xd61f0020] {
        // ldr/str x0,[x1]; br x1
        let mut memory = context_memory(word);
        let mut cpu = patterned_state(7);
        cpu.registers[1] = VECTOR_VA + 0xf000;
        assert!(
            matches!(e.run(&mut cpu, &mut memory, Duration::from_millis(2)),
            Err(Error::Memory(MemoryError::Unmapped(address))) if address == VECTOR_VA + 0xf000)
        );
    }
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn sparse_page_table_growth_removal_and_reused_gpas_remain_exact() {
    let mut executor = Executor::new().unwrap();
    let mut memory = memory();
    svc(&mut executor, &mut state(), &mut memory);
    let a = executor.machine.allocate(None).unwrap();
    let b = executor.machine.allocate(None).unwrap();
    for (page, value) in [(&a, 41u64), (&b, 901)] {
        page.begin_host_access();
        // SAFETY: this test owns the complete slot; its host gate excludes HVF.
        unsafe { page.pointer().cast::<u64>().write(value) };
        page.end_host_access();
    }
    // Each address requires a different leaf table. Include reuse, random order,
    // both ends of the sparse span and root teardown with many tables still live.
    const COUNT: u64 = 1024;
    let address = |index: u64| 0x2000_0000_0000 + index * (2 << 20);
    for index in 0..COUNT {
        executor
            .pte(address(index), a.physical_address(), 3)
            .unwrap();
    }
    for index in (0..COUNT).step_by(2) {
        executor.pte(address(index), 0, 0).unwrap();
    }
    for index in (0..COUNT).rev().filter(|index| index % 2 == 0) {
        executor
            .pte(address(index), b.physical_address(), 3)
            .unwrap();
    }
    let mut expected = [41u64, 901];
    for index in (0..COUNT).map(|index| (index * 509) % COUNT) {
        let which = usize::from(index % 2 == 0);
        let mut registers = state();
        registers.registers[0] = address(index);
        svc(&mut executor, &mut registers, &mut memory);
        expected[which] += 1;
        assert_eq!(registers.registers[1], expected[which]);
    }
    for index in (0..COUNT).map(|index| (index * 509) % COUNT) {
        executor.pte(address(index), 0, 0).unwrap();
    }
    let mut registers = state();
    registers.registers[0] = address(COUNT - 1);
    assert!(matches!(
        executor.run(&mut registers, &mut memory, Duration::from_millis(20)),
        Err(Error::Memory(MemoryError::Unmapped(_)))
    ));
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn anonymous_pager_preserves_host_edits_and_releases_final_storage() {
    let machine = Machine::shared().unwrap();
    let before = machine.stats();
    let pages: Vec<_> = (0..8).map(|_| machine.allocate(None).unwrap()).collect();
    assert_eq!(machine.stats().backing_bytes, before.backing_bytes + 32768);
    for round in 0..8u64 {
        reclaim(u64::MAX).unwrap();
        for (index, page) in pages.iter().enumerate() {
            page.begin_host_access();
            // SAFETY: retained live slot and synchronous host exclusion gate.
            unsafe {
                let pointer = page.pointer().cast::<u64>();
                let expected = if round == 0 { 0 } else { round + index as u64 };
                assert_eq!(pointer.read(), expected);
                pointer.write(round + index as u64 + 1);
            }
            page.end_host_access();
            assert!(machine.resume(page.physical_address()).unwrap().is_none());
        }
    }
    drop(pages);
    assert_eq!(machine.stats().backing_bytes, before.backing_bytes);
    assert_eq!(machine.stats().mapped_bytes, before.mapped_bytes);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn migrating_workers_reuse_resident_translation_blocks_without_new_page_faults() {
    let mut memory = memory();
    memory
        .map(0x40000, PAGE_SIZE * 64, Permissions::RW)
        .unwrap();
    let mut first = Executor::new().unwrap();
    for page in 0..64 {
        let mut registers = state();
        registers.registers[0] = 0x40000 + page * PAGE_SIZE;
        svc(&mut first, &mut registers, &mut memory);
        assert_eq!(registers.registers[1], 1);
    }
    let before = paging_stats().backing_bytes;
    let resident = memory.resident_bytes();
    let _machine = first.machine.clone();
    drop(first); // Hypervisor allows one live vCPU on its creator thread.
    let mut second = Executor::new().unwrap();
    let mut registers = state();
    registers.registers[0] = 0x40000;
    svc(&mut second, &mut registers, &mut memory);
    let faults = second.translations.faults;
    for page in 1..64 {
        let mut registers = state();
        registers.registers[0] = 0x40000 + page * PAGE_SIZE;
        svc(&mut second, &mut registers, &mut memory);
        assert_eq!(registers.registers[1], 2);
    }
    assert_eq!(second.translations.faults, faults);
    assert_eq!(paging_stats().backing_bytes, before);
    assert_eq!(memory.resident_bytes(), resident);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn anonymous_arenas_reuse_only_retired_granules_and_zero_new_slots() {
    let machine = Machine::shared().unwrap();
    let before = machine.stats();
    let mut pages: Vec<_> = (0..256)
        .map(|_| Some(machine.allocate(None).unwrap()))
        .collect();
    let base = pages[0].as_ref().unwrap().pointer() as usize;
    for (index, page) in pages.iter().enumerate() {
        let page = page.as_ref().unwrap();
        assert_eq!(page.pointer() as usize, base + index * PAGE_SIZE as usize);
        page.begin_host_access();
        // SAFETY: the owned slot's gate excludes hardware byte access.
        unsafe { page.pointer().cast::<u64>().write(index as u64 + 1) };
        page.end_host_access();
    }
    for page in &mut pages[128..132] {
        *page = None;
    }
    let reused: Vec<_> = (0..4).map(|_| machine.allocate(None).unwrap()).collect();
    for (index, page) in reused.iter().enumerate() {
        assert_eq!(
            page.pointer() as usize,
            base + (128 + index) * PAGE_SIZE as usize
        );
        page.begin_host_access();
        unsafe { assert_eq!(page.pointer().cast::<u64>().read(), 0) };
        page.end_host_access();
    }
    for (index, page) in pages.iter().enumerate() {
        if let Some(page) = page {
            page.begin_host_access();
            unsafe { assert_eq!(page.pointer().cast::<u64>().read(), index as u64 + 1) };
            page.end_host_access();
        }
    }
    drop(reused);
    drop(pages);
    assert_eq!(machine.stats().backing_bytes, before.backing_bytes);
    assert_eq!(machine.stats().mapped_bytes, before.mapped_bytes);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn large_untouched_vmas_do_not_rescan_unrelated_cached_hardware_pages() {
    let mut memory = memory();
    memory
        .map(0x40000, 64 * PAGE_SIZE, Permissions::RW)
        .unwrap();
    let mut executor = Executor::new().unwrap();
    for page in 0..64 {
        let mut state = state();
        state.registers[0] = 0x40000 + page * PAGE_SIZE;
        svc(&mut executor, &mut state, &mut memory);
    }
    let before = executor.translations;
    let resident = memory.resident_bytes();
    memory
        .map(0x100000, 8192 * PAGE_SIZE, Permissions::RW)
        .unwrap();
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(executor.translations.inspections, before.inspections);
    assert_eq!(memory.resident_bytes(), resident);
    memory.unmap(0x100000, 8192 * PAGE_SIZE).unwrap();
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(executor.translations.inspections, before.inspections);
    memory.protect(0x40000, PAGE_SIZE, Permissions::R).unwrap();
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(executor.translations.inspections, before.inspections + 1);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn late_worker_revalidates_only_changed_chunks_after_journal_expiry() {
    const CHANGED: u64 = 0x400000;
    const COLD: u64 = 0x80000000;
    let mut memory = memory();
    for base in [0x40000, CHANGED] {
        memory.map(base, 64 * PAGE_SIZE, Permissions::RW).unwrap();
    }
    memory.map(COLD, PAGE_SIZE, Permissions::RW).unwrap();
    let mut executor = Executor::new().unwrap();
    for base in [0x40000, CHANGED] {
        for page in 0..64 {
            let mut cpu = state();
            cpu.registers[0] = base + page * PAGE_SIZE;
            svc(&mut executor, &mut cpu, &mut memory);
        }
    }
    let before = executor.translations;
    let resident = memory.resident_bytes();
    let update_cold = |memory: &mut Memory, executor: &mut Executor| {
        for i in 0..80 {
            memory
                .protect(
                    COLD,
                    PAGE_SIZE,
                    if i % 2 == 0 {
                        Permissions::R
                    } else {
                        Permissions::RW
                    },
                )
                .unwrap();
            drop(memory.parallel_view(&mut executor.registry).unwrap());
            memory.commit_parallel().unwrap();
        }
    };
    update_cold(&mut memory, &mut executor);
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(executor.translations.inspections, before.inspections);
    memory.protect(CHANGED, PAGE_SIZE, Permissions::R).unwrap();
    update_cold(&mut memory, &mut executor);
    svc(&mut executor, &mut state(), &mut memory);
    assert_eq!(executor.translations.inspections, before.inspections + 64);
    let mut cpu = state();
    cpu.registers[0] = CHANGED;
    assert!(matches!(
        executor.run(&mut cpu, &mut memory, Duration::from_millis(20)),
        Err(Error::Memory(MemoryError::Permission { .. }))
    ));
    memory.unmap(CHANGED, 64 * PAGE_SIZE).unwrap();
    update_cold(&mut memory, &mut executor);
    svc(&mut executor, &mut state(), &mut memory);
    let mut cpu = state();
    cpu.registers[0] = CHANGED;
    assert!(matches!(
        executor.run(&mut cpu, &mut memory, Duration::from_millis(20)),
        Err(Error::Memory(MemoryError::Unmapped(_)))
    ));
    memory.map(CHANGED, PAGE_SIZE, Permissions::RW).unwrap();
    update_cold(&mut memory, &mut executor);
    let mut cpu = state();
    cpu.registers[0] = CHANGED;
    svc(&mut executor, &mut cpu, &mut memory);
    assert_eq!(
        cpu.registers[1], 1,
        "old bytes/permissions cannot survive remap"
    );
    assert!(memory.resident_bytes() <= resident);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn private_arena_addresses_stay_linear_across_table_growth_and_release_without_ram_cache() {
    let executor = Executor::new().unwrap();
    let machine = executor.machine.clone();
    let mut pages = Vec::new();
    let mut base = 0;
    for index in 0..256u64 {
        let page = machine.allocate(None).unwrap();
        if index == 0 {
            base = page.physical_address();
            assert_eq!(machine.stats().mapped_bytes, 16384);
            assert_eq!(machine.stats().backing_bytes, 16384);
        }
        assert_eq!(page.physical_address(), base + index * PAGE_SIZE);
        // Each leaf allocates new page tables from the independent 16KiB pool.
        executor
            .pte(0x100000000 + index * (2 << 20), page.physical_address(), 3)
            .unwrap();
        pages.push(page);
    }
    for index in 0..256u64 {
        executor.pte(0x100000000 + index * (2 << 20), 0, 0).unwrap();
    }
    drop(pages);
    assert_eq!(machine.stats().mapped_bytes, 0);
    assert_eq!(machine.stats().backing_bytes, 0);
    let reused = machine.allocate(None).unwrap();
    assert_eq!(reused.physical_address(), base);
    assert_eq!(machine.stats().mapped_bytes, 16384);
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn fresh_leaves_and_table_branches_publish_without_tlb_invalidation() {
    let mut memory = memory();
    memory
        .map(0x40000, 64 * PAGE_SIZE, Permissions::RW)
        .unwrap();
    let branches = [0x200000, 0x40000000, 0x8000000000];
    for &address in &branches {
        memory.map(address, PAGE_SIZE, Permissions::RW).unwrap();
    }
    let mut executor = Executor::new().unwrap();
    svc(&mut executor, &mut state(), &mut memory);
    let before = executor.register_stats();
    for page in 0..64 {
        let mut cpu = state();
        cpu.registers[0] = 0x40000 + page * PAGE_SIZE;
        svc(&mut executor, &mut cpu, &mut memory);
        assert_eq!(cpu.registers[1], 1);
    }
    for &address in &branches {
        let mut cpu = state();
        cpu.registers[0] = address;
        svc(&mut executor, &mut cpu, &mut memory);
        assert_eq!(cpu.registers[1], 1);
    }
    let after = executor.register_stats();
    assert!(after.publications > before.publications);
    assert_eq!(after.invalidations, before.invalidations);
    assert_eq!(after.full_invalidations, before.full_invalidations);
    assert_eq!(
        after.entries - before.entries,
        after.mailbox - before.mailbox + after.fallback - before.fallback + after.completions
            - before.completions
            + after.restore_interruptions
            - before.restore_interruptions,
        "no extra TLB-only hypervisor entries"
    );
}

#[test]
#[ignore = "requires Hypervisor entitlement and Apple Silicon"]
fn demand_faults_continue_the_same_timer_and_complete_register_bank() {
    let mut memory = context_memory(0xd4000001);
    memory
        .map(0x40000, 64 * PAGE_SIZE, Permissions::RW)
        .unwrap();
    // Each load crosses to a fresh page, then one final SVC. x0 moves through
    // the range, x1 is zero and x2 counts down; every other GPR/SIMD bit survives.
    for (i, word) in [
        0xf9400001u32,
        0x91400400,
        0xf1000442,
        0x54ffffa1,
        0xd4000001,
    ]
    .into_iter()
    .enumerate()
    {
        memory.protect(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
        memory
            .write_array(0x10000 + i as u64 * 4, word.to_le_bytes())
            .unwrap();
    }
    memory.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    let mut executor = Executor::new().unwrap();
    let mut cpu = patterned_state(42);
    cpu.registers[0] = 0x40000;
    cpu.registers[2] = 64;
    let before = executor.register_stats();
    let mut expected = cpu.clone();
    expected.pc += 20;
    expected.registers[0] += 64 * PAGE_SIZE;
    expected.registers[1] = 0;
    expected.registers[2] = 0;
    expected.nzcv = 0x60000000;
    svc(&mut executor, &mut cpu, &mut memory);
    assert_eq!(cpu, expected);
    let after = executor.register_stats();
    assert!(after.fault_continuations - before.fault_continuations >= 32);
    assert!(
        after.timer_programs - before.timer_programs < 16,
        "a fresh timer is not programmed for every demand fault: {after:?}"
    );
}
