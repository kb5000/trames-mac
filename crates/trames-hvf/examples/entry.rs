// SPDX-License-Identifier: GPL-3.0-or-later
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use trames_execution::{Aarch64State, ExitReason, Registers};
    use trames_memory::{Memory, Permissions};
    let mut memory = Memory::default();
    memory.map(0x10000, 4096, Permissions::RW)?;
    let code: [u32; 6] = [
        0xd2801588, 0xd4000001, 0x9100a400, 0xf9000120, 0xd2800ba8, 0xd4000001,
    ];
    memory.write(
        0x10000,
        &code
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    memory.protect(0x10000, 4096, Permissions::RX)?;
    memory.map(0x100000000, 4096, Permissions::RW)?;
    let mut state = Aarch64State {
        pc: 0x10000,
        ..Default::default()
    };
    state.set_x(9, 0x100000000);
    let mut linux = trames_linux::Linux::new(0x20000);
    let mut output = trames_linux::CapturedOutput::default();
    let mut engine = trames_hvf::Executor::new()?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut syscalls = 0;
    loop {
        assert!(
            std::time::Instant::now() < end,
            "hardware guest did not finish"
        );
        if engine
            .run(&mut state, &mut memory, std::time::Duration::from_millis(2))?
            .reason
            == ExitReason::Syscall
        {
            syscalls += 1;
            match linux.dispatch(&mut state, &mut memory, &mut output) {
                trames_linux::SyscallOutcome::Exit(code) => {
                    assert_eq!(code, 42);
                    break;
                }
                trames_linux::SyscallOutcome::Continue => {}
            }
        }
    }
    assert_eq!(syscalls, 2);
    assert_eq!(memory.read_array::<8>(0x100000000)?, 42u64.to_le_bytes());
    let child = memory.clone();
    state.pc = 0x1000c; // STR, after the completed getpid
    state.set_x(0, 99);
    loop {
        assert!(std::time::Instant::now() < end);
        if engine
            .run(&mut state, &mut memory, std::time::Duration::from_millis(2))?
            .reason
            == ExitReason::Syscall
        {
            break;
        }
    }
    assert_eq!(memory.read_array::<8>(0x100000000)?, 99u64.to_le_bytes());
    assert_eq!(child.read_array::<8>(0x100000000)?, 42u64.to_le_bytes());
    drop(child);
    memory.protect(0x100000000, 4096, Permissions::R)?;
    state.pc = 0x1000c;
    state.set_x(0, 88);
    assert!(matches!(
        engine.run(&mut state, &mut memory, std::time::Duration::from_millis(2)),
        Err(trames_hvf::Error::Memory(
            trames_memory::MemoryError::Permission { .. }
        ))
    ));
    assert_eq!(memory.read_array::<8>(0x100000000)?, 99u64.to_le_bytes());
    let before = engine.mapped_bytes();
    memory.unmap(0x100000000, 4096)?;
    engine.synchronize(&memory)?;
    assert!(engine.mapped_bytes() < before);
    memory.map(0x30000, 4096, Permissions::RW)?;
    memory.write_array(0x30000, 0x14000000u32.to_le_bytes())?;
    memory.protect(0x30000, 4096, Permissions::RX)?;
    state.pc = 0x30000;
    let began = std::time::Instant::now();
    let yielded = engine.run(&mut state, &mut memory, std::time::Duration::from_millis(2))?;
    assert_eq!(yielded.reason, ExitReason::Yield);
    assert_eq!(
        yielded.instructions,
        trames_execution::InstructionCount::Unavailable
    );
    assert!(began.elapsed() < std::time::Duration::from_secs(1));
    println!("PASS: real EL0 ARM64, SVC -> common Linux getpid/exit, sparse first-write mapping, fork COW, write protection, live VM reclamation and bounded timer yield");
    Ok(())
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("This execution probe requires Apple Silicon macOS.");
}
