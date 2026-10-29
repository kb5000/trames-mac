// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
core::arch::global_asm!(include_str!("sme_tests.S"), options(raw));
unsafe extern "C" {
    static trames_sme_test_begin: u8;
    static trames_sme_test_end: u8;
    static trames_sme_yield_begin: u8;
    static trames_sme_yield_loop: u8;
    static trames_sme_yield_finish: u8;
    static trames_sme_yield_end: u8;
}
fn code(start: *const u8, end: *const u8) -> Memory {
    let mut m = memory();
    m.protect(0x10000, PAGE_SIZE, Permissions::RW).unwrap();
    // SAFETY: linked immutable fixture labels delimit one contiguous text
    // region. The copied code has only relative branches and guest pointers.
    let bytes = unsafe { std::slice::from_raw_parts(start, end as usize - start as usize) };
    m.write(0x10000, bytes).unwrap();
    m.protect(0x10000, PAGE_SIZE, Permissions::RX).unwrap();
    m.write(0x20000, &[0xa7; 64]).unwrap();
    m
}
fn matrix(r: &SmeState, expected: f32) {
    assert_eq!(r.vl, 64);
    let mut count = 0;
    for bytes in r.za.chunks_exact(4) {
        let value = f32::from_le_bytes(bytes.try_into().unwrap());
        if value != 0.0 {
            assert_eq!(value, expected);
            count += 1;
        }
    }
    assert_eq!(count, 256);
    assert_eq!(r.zt, [0xa7; 64]);
}
#[test]
#[ignore = "requires Hypervisor entitlement and SME2"]
fn sme_syscall_and_cross_thread_migration_preserve_za_zt_and_tls() {
    if features().hwcap2 & (1 << 37) == 0 {
        eprintln!("SME2 unavailable");
        return;
    }
    let mut executor = Executor::new().unwrap();
    let mut m = code(
        &raw const trames_sme_test_begin,
        &raw const trames_sme_test_end,
    );
    let mut registry = Registry::default();
    registry.use_hardware();
    m.prepare_hardware().unwrap();
    let mut view = m.parallel_view(&mut registry).unwrap();
    view.enable_hardware();
    let mut s = state();
    let mut r = None;
    loop {
        if executor
            .run_parallel_sme(&mut s, &mut r, 64, &mut view, Duration::from_millis(2))
            .unwrap()
            .reason
            == ExitReason::Syscall
        {
            break;
        }
    }
    assert_eq!(s.registers[2], 64);
    assert_eq!(s.tpidr2_el0, 0x713);
    assert_eq!(r.as_ref().unwrap().svcr, 2);
    matrix(r.as_ref().unwrap(), 2.0);
    assert_eq!(
        s.vectors[0],
        u128::from_le_bytes(1f32.to_le_bytes().repeat(4).try_into().unwrap())
    );
    let (s, r, view) = std::thread::spawn(move || {
        let mut executor = Executor::new().unwrap();
        loop {
            if executor
                .run_parallel_sme(&mut s, &mut r, 64, &mut view, Duration::from_millis(2))
                .unwrap()
                .reason
                == ExitReason::Syscall
            {
                break;
            }
        }
        (s, r, view)
    })
    .join()
    .unwrap();
    matrix(r.as_ref().unwrap(), 4.0);
    assert_eq!(s.registers[3], 0x713);
    drop(view);
    m.commit_parallel().unwrap();
    assert_eq!(m.read_array::<64>(0x20040).unwrap(), [0xa7; 64]);
}
#[test]
#[ignore = "requires Hypervisor entitlement and SME2"]
fn sme_timer_exit_preserves_streaming_vectors_and_predicates() {
    if features().hwcap2 & (1 << 37) == 0 {
        eprintln!("SME2 unavailable");
        return;
    }
    let mut executor = Executor::new().unwrap();
    let mut m = code(
        &raw const trames_sme_yield_begin,
        &raw const trames_sme_yield_end,
    );
    let mut registry = Registry::default();
    registry.use_hardware();
    m.prepare_hardware().unwrap();
    let mut view = m.parallel_view(&mut registry).unwrap();
    view.enable_hardware();
    let mut s = state();
    let mut r = None;
    let target = 0x10000
        + (&raw const trames_sme_yield_loop as u64 - &raw const trames_sme_yield_begin as u64);
    for _ in 0..100 {
        assert_eq!(
            executor
                .run_parallel_sme(&mut s, &mut r, 64, &mut view, Duration::from_millis(2))
                .unwrap()
                .reason,
            ExitReason::Yield
        );
        if s.pc == target {
            break;
        }
    }
    assert_eq!(s.pc, target);
    assert_eq!(s.tpidr2_el0, 0x713);
    let original = r.as_ref().unwrap();
    assert_eq!(original.svcr, 3);
    matrix(original, 2.0);
    assert_eq!(&original.z[..64], 1f32.to_le_bytes().repeat(16));
    assert_eq!(&original.z[64..128], 2f32.to_le_bytes().repeat(16));
    assert_eq!(&original.p[..8], &[0x11; 8]);
    s.pc = 0x10000
        + (&raw const trames_sme_yield_finish as u64 - &raw const trames_sme_yield_begin as u64);
    std::thread::spawn(move || {
        let mut executor = Executor::new().unwrap();
        loop {
            if executor
                .run_parallel_sme(&mut s, &mut r, 64, &mut view, Duration::from_millis(2))
                .unwrap()
                .reason
                == ExitReason::Syscall
            {
                break;
            }
        }
        assert_eq!(r.as_ref().unwrap().svcr, 2);
        matrix(r.as_ref().unwrap(), 4.0);
    })
    .join()
    .unwrap();
}
