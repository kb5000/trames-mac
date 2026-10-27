// SPDX-License-Identifier: GPL-3.0-or-later
//! Task state may move; the HVF vCPU remains on its creator host thread.
use crate::{Error, Executor};
use std::time::Duration;
use trames_engine::{CpuFault, DecodeStats, SliceOutcome};
use trames_execution::{Aarch64State, ExecutionExit, ExitReason, Features, Registers};
use trames_memory::{parallel::View, CpuMemory, Memory, MemoryError};

#[derive(Clone, Default)]
pub struct Cpu {
    pub state: Aarch64State,
    vulkan_calls: bool,
}

#[derive(Default)]
pub struct Engine {
    executor: Option<Executor>,
}

impl Engine {
    pub fn mapped_bytes(&self) -> usize {
        self.executor.as_ref().map_or(0, Executor::mapped_bytes)
    }
}

impl Cpu {
    fn outcome(
        &self,
        result: Result<ExecutionExit, Error>,
        memory: &impl CpuMemory,
    ) -> SliceOutcome {
        match result {
            Ok(exit) => match exit.reason {
                ExitReason::Syscall => SliceOutcome::Syscall { instructions: 0 },
                ExitReason::Yield => SliceOutcome::Yield { instructions: 0 },
            },
            Err(error) => {
                if self.vulkan_calls
                    && self.state.x(16) == trames_accel::native::VULKAN_PROFILE
                    && matches!(&error, Error::Fault { pc, syndrome, .. }
                        if *pc == self.state.pc && syndrome >> 26 == 0)
                {
                    // HLT from EL0 is undefined under HVF. Recognize only the
                    // negotiated opcode/profile; the common registry still
                    // authorizes the complete five-word stub and operation.
                    match memory.fetch_u32(self.state.pc) {
                        Ok(trames_accel::native::INSTRUCTION) => {
                            return SliceOutcome::NativeCall {
                                instructions: 0,
                                pc: self.state.pc,
                                module: self.state.x(16),
                                function: self.state.x(17),
                            };
                        }
                        Err(error) => {
                            return SliceOutcome::Fault(fault(&self.state, Error::Memory(error)));
                        }
                        _ => {}
                    }
                }
                SliceOutcome::Fault(fault(&self.state, error))
            }
        }
    }
}

impl Registers for Cpu {
    fn x(&self, index: usize) -> u64 {
        self.state.x(index)
    }
    fn set_x(&mut self, index: usize, value: u64) {
        self.state.set_x(index, value);
    }
    fn pc(&self) -> u64 {
        self.state.pc
    }
}

fn fault(state: &Aarch64State, error: Error) -> CpuFault {
    match error {
        Error::Memory(MemoryError::BackingIo(95) | MemoryError::InvalidRange) => {
            CpuFault::Host(error.to_string())
        }
        Error::Memory(error) => CpuFault::Memory {
            pc: state.pc,
            error,
        },
        Error::Fault { pc, syndrome, .. } => {
            let (signal, code, address) = match syndrome >> 26 {
                // Undefined instructions and trapped privileged system-register
                // accesses are recoverable Linux SIGILL, not backend failures.
                0 | 0x18 => (4, 1, pc),
                0x3c => (5, 1, pc),
                0x22 | 0x26 => {
                    return CpuFault::Memory {
                        pc,
                        error: MemoryError::Alignment(if syndrome >> 26 == 0x26 {
                            state.sp
                        } else {
                            pc
                        }),
                    }
                }
                _ => return CpuFault::Host(error.to_string()),
            };
            // The shared kernel preserves the fault address in Linux siginfo.
            CpuFault::Trap {
                pc: address,
                instruction: 0,
                signal,
                code,
            }
        }
        _ => CpuFault::Host(error.to_string()),
    }
}

impl trames_engine::Cpu for Cpu {
    type Engine = Engine;
    const FEATURES: Features = Features {
        hwcap: 3,
        hwcap2: 0,
    };
    const FEATURE_NAMES: &'static str = "fp asimd";
    // Every worker owns its thread-affine vCPU; physical bytes are shared
    // through the common frozen View and its hardware host-borrow gate.
    const PARALLEL: bool = true;
    const EXACT_INSTRUCTIONS: bool = false;
    fn new(pc: u64, sp: u64) -> Self {
        Self {
            state: Aarch64State {
                pc,
                sp,
                ..Default::default()
            },
            vulkan_calls: false,
        }
    }
    fn sp(&self) -> u64 {
        self.state.sp
    }
    fn set_sp(&mut self, value: u64) {
        self.state.sp = value;
    }
    fn set_pc(&mut self, value: u64) {
        self.state.pc = value;
    }
    fn set_tls(&mut self, value: u64) {
        self.state.tpidr_el0 = value;
    }
    fn architectural_state(&self) -> Aarch64State {
        self.state.clone()
    }
    fn restore_architectural_state(&mut self, state: &Aarch64State) {
        self.state.clone_from(state);
    }
    fn set_native_calls(&mut self, enabled: bool) {
        assert!(!enabled, "HVF executes the original native instructions");
    }
    fn set_vulkan_calls(&mut self, enabled: bool) {
        self.vulkan_calls = enabled;
    }
    fn clear_exclusive(&mut self) {}
    fn fault_retired(&self) -> u64 {
        0
    }
    fn decode_stats(&self) -> DecodeStats {
        DecodeStats::default()
    }
    fn run(&mut self, engine: &mut Engine, memory: &mut Memory, _: u64) -> SliceOutcome {
        if engine.executor.is_none() {
            match Executor::new() {
                Ok(executor) => engine.executor = Some(executor),
                Err(error) => return SliceOutcome::Fault(CpuFault::Host(error.to_string())),
            }
        }
        let result = engine.executor.as_mut().unwrap().run(
            &mut self.state,
            memory,
            Duration::from_millis(2),
        );
        self.outcome(result, memory)
    }
    fn run_coordinated(
        &mut self,
        engine: &mut Engine,
        memory: &mut Memory,
        registry: &mut trames_memory::parallel::Registry,
        _: u64,
    ) -> SliceOutcome {
        if engine.executor.is_none() {
            match Executor::new() {
                Ok(executor) => engine.executor = Some(executor),
                Err(error) => return SliceOutcome::Fault(CpuFault::Host(error.to_string())),
            }
        }
        let result = (|| {
            Self::prepare_hardware_memory(memory, registry)?;
            let mut view = memory
                .parallel_view(registry)
                .ok_or(Error::Unsupported("hardware view admission"))?;
            view.enable_hardware();
            let result = engine.executor.as_mut().unwrap().run_parallel(
                &mut self.state,
                &mut view,
                Duration::from_millis(2),
            );
            drop(view);
            memory.commit_parallel()?;
            result
        })();
        self.outcome(result, memory)
    }
    fn prepare_hardware_memory(
        memory: &mut Memory,
        registry: &mut trames_memory::parallel::Registry,
    ) -> Result<(), MemoryError> {
        registry.use_hardware();
        memory.prepare_hardware()
    }
    fn prepare_hardware_view(view: &mut View) {
        view.enable_hardware();
    }
    fn take_hardware_wait(
        engine: &mut Engine,
    ) -> Option<std::sync::Arc<trames_memory::HardwareWait>> {
        engine.executor.as_mut().and_then(Executor::take_wait)
    }
    fn run_parallel(&mut self, engine: &mut Engine, memory: &mut View, _: u64) -> SliceOutcome {
        if engine.executor.is_none() {
            match Executor::new() {
                Ok(executor) => engine.executor = Some(executor),
                Err(error) => return SliceOutcome::Fault(CpuFault::Host(error.to_string())),
            }
        }
        let result = engine.executor.as_mut().unwrap().run_parallel(
            &mut self.state,
            memory,
            Duration::from_millis(2),
        );
        self.outcome(result, memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trames_engine::Cpu as _;

    fn native_stub() -> (Cpu, Memory) {
        let mut memory = Memory::default();
        memory
            .map(0x1000, 4096, trames_memory::Permissions::RW)
            .unwrap();
        for (index, word) in [
            0xd28000d0u32,
            0xf2a00030,
            0xd2800031,
            trames_accel::native::INSTRUCTION,
            0xd65f03c0,
        ]
        .into_iter()
        .enumerate()
        {
            memory
                .write_array(0x1000 + index as u64 * 4, word.to_le_bytes())
                .unwrap();
        }
        memory
            .protect(0x1000, 4096, trames_memory::Permissions::RX)
            .unwrap();
        let mut cpu = Cpu::new(0x100c, 0);
        cpu.set_x(16, trames_accel::native::VULKAN_PROFILE);
        cpu.set_x(17, 1);
        (cpu, memory)
    }
    fn undefined(pc: u64) -> Result<ExecutionExit, Error> {
        Err(Error::Fault {
            pc,
            address: 0,
            syndrome: 0,
        })
    }
    #[test]
    fn native_transport_is_opt_in_and_keeps_common_authorization() {
        let (mut cpu, mut memory) = native_stub();
        assert!(matches!(
            cpu.outcome(undefined(cpu.pc()), &memory),
            SliceOutcome::Fault(CpuFault::Trap { signal: 4, .. })
        ));
        cpu.set_vulkan_calls(true);
        assert!(matches!(
            cpu.outcome(undefined(cpu.pc()), &memory),
            SliceOutcome::NativeCall {
                pc: 0x100c,
                module: 0x10006,
                function: 1,
                instructions: 0
            }
        ));
        let mut registry = trames_accel::native::Registry::default();
        assert!(registry
            .authorize_stateful(&memory, cpu.pc(), cpu.x(16), cpu.x(17))
            .is_ok());
        // A matching HLT is transport only; a modified prefix grants no host authority.
        memory
            .protect(0x1000, 4096, trames_memory::Permissions::RW)
            .unwrap();
        memory
            .write_array(0x1000, 0xd503201fu32.to_le_bytes())
            .unwrap();
        memory
            .protect(0x1000, 4096, trames_memory::Permissions::RX)
            .unwrap();
        assert!(registry
            .authorize_stateful(&memory, cpu.pc(), cpu.x(16), cpu.x(17))
            .is_err());
        cpu.set_x(16, trames_accel::native::PROFILE);
        assert!(matches!(
            cpu.outcome(undefined(cpu.pc()), &memory),
            SliceOutcome::Fault(_)
        ));
    }
    #[test]
    fn native_transport_preserves_other_traps_and_execute_permission() {
        let (mut cpu, mut memory) = native_stub();
        cpu.set_vulkan_calls(true);
        assert!(matches!(
            cpu.outcome(
                Err(Error::Fault {
                    pc: cpu.pc(),
                    address: 0,
                    syndrome: 0x3c << 26
                }),
                &memory
            ),
            SliceOutcome::Fault(CpuFault::Trap { signal: 5, .. })
        ));
        memory
            .protect(0x1000, 4096, trames_memory::Permissions::RW)
            .unwrap();
        assert!(matches!(
            cpu.outcome(undefined(cpu.pc()), &memory),
            SliceOutcome::Fault(CpuFault::Memory { .. })
        ));
        memory
            .write_array(0x100c, 0xd4400000u32.to_le_bytes())
            .unwrap();
        memory
            .protect(0x1000, 4096, trames_memory::Permissions::RX)
            .unwrap();
        assert!(matches!(
            cpu.outcome(undefined(cpu.pc()), &memory),
            SliceOutcome::Fault(CpuFault::Trap { signal: 4, .. })
        ));
    }
    #[test]
    #[ignore = "requires Hypervisor entitlement and Apple Silicon"]
    fn hardware_native_call_returns_through_shared_dispatch_boundary() {
        let (mut cpu, memory) = native_stub();
        let mut memory = memory;
        cpu.set_pc(0x1000);
        cpu.set_vulkan_calls(true);
        let mut engine = Engine::default();
        let result = loop {
            let result = cpu.run(&mut engine, &mut memory, u64::MAX);
            if !matches!(result, SliceOutcome::Yield { .. }) {
                break result;
            }
        };
        assert!(matches!(
            result,
            SliceOutcome::NativeCall {
                pc: 0x100c,
                module: 0x10006,
                function: 1,
                ..
            }
        ));
        let mut registry = trames_accel::native::Registry::default();
        registry
            .authorize_stateful(&memory, cpu.pc(), cpu.x(16), cpu.x(17))
            .unwrap();
    }

    #[test]
    fn privileged_system_register_access_delivers_guest_sigill() {
        let state = Aarch64State {
            pc: 0x1000,
            ..Default::default()
        };
        assert!(matches!(
            fault(
                &state,
                Error::Fault {
                    pc: state.pc,
                    address: 0xdead,
                    syndrome: 0x18 << 26,
                }
            ),
            CpuFault::Trap {
                pc: 0x1000,
                signal: 4,
                code: 1,
                ..
            }
        ));
    }
    #[test]
    fn stack_alignment_keeps_instruction_pc_separate_from_fault_address() {
        let state = Aarch64State {
            pc: 0x1000,
            sp: 0x2008,
            ..Default::default()
        };
        let failure = fault(
            &state,
            Error::Fault {
                pc: state.pc,
                address: state.sp,
                syndrome: 0x26 << 26,
            },
        );
        assert!(matches!(
            failure,
            CpuFault::Memory {
                pc: 0x1000,
                error: MemoryError::Alignment(0x2008)
            }
        ));
    }
}
