// SPDX-License-Identifier: GPL-3.0-or-later
//! Task state may move; the HVF vCPU remains on its creator host thread.
use crate::{Error, Executor};
use std::time::Duration;
use trames_engine::{CpuFault, DecodeStats, SliceOutcome};
use trames_execution::{Aarch64State, ExitReason, Features, Registers};
use trames_memory::{parallel::View, Memory, MemoryError};

#[derive(Clone, Default)]
pub struct Cpu {
    pub state: Aarch64State,
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
                0 => (4, 1, pc),
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
        assert!(!enabled, "HVF Vulkan bridge is not enabled");
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
        match engine.executor.as_mut().unwrap().run(
            &mut self.state,
            memory,
            Duration::from_millis(2),
        ) {
            Ok(exit) => match exit.reason {
                ExitReason::Syscall => SliceOutcome::Syscall { instructions: 0 },
                ExitReason::Yield => SliceOutcome::Yield { instructions: 0 },
            },
            Err(error) => SliceOutcome::Fault(fault(&self.state, error)),
        }
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
        match engine.executor.as_mut().unwrap().run_parallel(
            &mut self.state,
            memory,
            Duration::from_millis(2),
        ) {
            Ok(exit) => match exit.reason {
                ExitReason::Syscall => SliceOutcome::Syscall { instructions: 0 },
                ExitReason::Yield => SliceOutcome::Yield { instructions: 0 },
            },
            Err(error) => SliceOutcome::Fault(fault(&self.state, error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
