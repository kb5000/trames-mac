// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-task product routing over the common Linux signal implementation.
//! Task-group permissions, namespaces and multi-process routing stay an explicit
//! integration gate; these calls address this bring-up runner's sole task.
use trames_execution::{Aarch64State, Registers};
use trames_kernel::signal::{self, Disposition, Fault, Info, Signals};
use trames_memory::{Memory, MemoryError};
use trames_vfs::Errno;

#[derive(Default)]
pub struct Task {
    pub signals: Signals,
    pub stopped: bool,
}
impl Task {
    pub fn call(&mut self, state: &mut Aarch64State, memory: &mut Memory, uid: u32) -> bool {
        let a = std::array::from_fn(|i| state.x(i));
        let number = state.x(8);
        let result = match number {
            172 | 178 => Ok(1),
            132 => self.signals.alt_stack.call(state.sp, a[0], a[1], memory),
            134 => self.signals.action_call(a, memory).map(|_| 0),
            135 => self.signals.mask_call(a, memory),
            136 => {
                if a[1] != 8 {
                    Err(Errno(22))
                } else {
                    memory
                        .write(
                            a[0],
                            &(self.signals.pending_bits() & self.signals.mask).to_le_bytes(),
                        )
                        .map(|_| 0)
                        .map_err(|_| Errno(14))
                }
            }
            139 => {
                // rt_sigreturn restores x0 too; do not overwrite its result.
                if let Err(error) = self.signals.restore(state, memory) {
                    state.set_x(0, (-i64::from(error.0)) as u64);
                }
                return true;
            }
            129..=131 => self.send(number, a, uid),
            _ => return false,
        };
        state.set_x(0, result.unwrap_or_else(|e| (-i64::from(e.0)) as u64));
        true
    }
    fn send(&mut self, number: u64, a: [u64; 6], uid: u32) -> Result<u64, Errno> {
        let (target, sig) = match number {
            129 => {
                let target = a[0] as i32;
                (matches!(target, 0 | 1), a[1] as u32)
            }
            130 => (a[0] as u32 == 1, a[1] as u32),
            _ => {
                if a[0] == 0 || a[0] > i32::MAX as u64 {
                    return Err(Errno(22));
                }
                (a[0] == 1 && a[1] as u32 == 1, a[2] as u32)
            }
        };
        if sig > 64 {
            return Err(Errno(22));
        }
        if !target {
            return Err(Errno(3));
        }
        if sig == 18 {
            self.stopped = false;
        }
        self.signals.clear_opposing(sig);
        if self.signals.should_queue(sig) {
            let info = Info {
                sender: 1,
                uid,
                code: if number == 129 { 0 } else { -6 },
                ..Default::default()
            };
            if number == 129 {
                self.signals
                    .shared_pending
                    .borrow_mut()
                    .enqueue(sig, info)?;
            } else {
                self.signals.pending.enqueue(sig, info)?;
            }
        }
        Ok(0)
    }
    pub fn fault(
        &mut self,
        state: &Aarch64State,
        error: &trames_hvf::Error,
    ) -> Result<Option<u8>, ()> {
        let fault = match error {
            // Backend admission failures are host errors, not guest signals.
            trames_hvf::Error::Memory(MemoryError::BackingIo(95) | MemoryError::InvalidRange) => {
                return Err(())
            }
            trames_hvf::Error::Memory(error) => Fault::memory(state.pc, error),
            trames_hvf::Error::Fault { pc, syndrome, .. } => match syndrome >> 26 {
                0 => Fault {
                    signal: 4,
                    code: 1,
                    address: *pc,
                },
                0x3c => Fault {
                    signal: 5,
                    code: 1,
                    address: *pc,
                },
                0x22 => Fault {
                    signal: 7,
                    code: 1,
                    address: *pc,
                },
                0x26 => Fault {
                    signal: 7,
                    code: 1,
                    address: state.sp,
                },
                _ => return Err(()),
            },
            _ => return Err(()),
        };
        Ok((!self.signals.force_fault(fault)).then_some(128 + fault.signal as u8))
    }
    pub fn deliver(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut Memory,
    ) -> Result<Option<u8>, Errno> {
        if let Some((sig, shared)) = self.signals.select(self.stopped, false) {
            let info = self.signals.take_signal(sig, shared);
            let action = self.signals.actions.get(sig as usize);
            match signal::disposition(sig, action) {
                Disposition::Terminate => return Ok(Some(128 + sig as u8)),
                Disposition::Stop => self.stopped = true,
                Disposition::Ignore => {}
                Disposition::Handler => self.signals.enter(state, memory, sig, info, action)?,
            }
        }
        Ok(None)
    }
}
