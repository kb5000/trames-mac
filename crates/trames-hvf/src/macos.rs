// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{Arc, Weak},
    time::Duration,
};
use trames_execution::{Aarch64State, ExecutionExit, ExitReason, Features, InstructionCount};
use trames_memory::{
    parallel::{Registry, View},
    Access, CpuMemory, HardwarePageInfo, HardwareStorage, HardwareWait, Memory, MemoryError,
    PAGE_SIZE,
};
#[path = "machine.rs"]
mod machine;
use machine::Machine;
pub use machine::{paging_stats, reclaim, set_resident_limit, take_memory_pressure, PagingStats};
const VECTOR_VA: u64 = 0xffffffe00000;
#[repr(C)]
#[derive(Default)]
struct HostExit {
    kind: u64,
    syndrome: u64,
    address: u64,
    pc: u64,
}
unsafe extern "C" {
    fn trames_hvf_max_vcpus() -> u32;
    fn trames_hvf_page(host: *mut c_void, va: u64, gpa: u64, flags: u32) -> i32;
    fn trames_hvf_map(host: *mut c_void, memory: *mut c_void, gpa: u64, size: usize) -> i32;
    fn trames_hvf_unmap(host: *mut c_void, gpa: u64, size: usize) -> i32;
    fn trames_hvf_icache(memory: *mut c_void, size: usize);
    fn trames_hvf_run(
        host: *mut c_void,
        state: *mut Aarch64State,
        nanos: u64,
        result: *mut HostExit,
    ) -> i32;
}
#[derive(Debug)]
pub enum Error {
    Host(i32),
    Io(std::io::Error),
    Memory(MemoryError),
    Unsupported(&'static str),
    Fault {
        pc: u64,
        address: u64,
        syndrome: u64,
    },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<MemoryError> for Error {
    fn from(value: MemoryError) -> Self {
        Self::Memory(value)
    }
}
fn check(value: i32) -> Result<(), Error> {
    if value == 0 {
        Ok(())
    } else {
        Err(Error::Host(value))
    }
}
struct Page {
    storage: Weak<dyn HardwareStorage>,
    gpa: u64,
    info: HardwarePageInfo,
}
/// A thread-affine vCPU with private stage-one tables in the process-wide VM.
/// Canonical physical pages, VFS and native access share one byte authority.
pub struct Executor {
    host: NonNull<c_void>,
    machine: Arc<Machine>,
    pages: BTreeMap<u64, Page>,
    registry: Registry,
    wait: Option<Arc<HardwareWait>>,
    _same_thread: PhantomData<Rc<()>>,
}
impl Executor {
    pub fn new() -> Result<Self, Error> {
        assert_eq!(std::mem::size_of::<Aarch64State>(), 816);
        let machine = Machine::shared()?;
        let host = machine.cpu()?;
        let mut registry = Registry::default();
        registry.use_hardware();
        Ok(Self {
            host,
            machine,
            pages: BTreeMap::new(),
            registry,
            wait: None,
            _same_thread: PhantomData,
        })
    }
    pub fn features(&self) -> Features {
        Features {
            hwcap: 3,
            hwcap2: 0,
        }
    }
    pub fn mapped_bytes(&self) -> usize {
        let granule = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
        self.pages
            .values()
            .map(|p| p.gpa & !(granule - 1))
            .collect::<BTreeSet<_>>()
            .len()
            * granule as usize
    }
    pub fn take_wait(&mut self) -> Option<Arc<HardwareWait>> {
        self.wait.take()
    }
    fn flags(info: HardwarePageInfo) -> Result<u32, Error> {
        if !info.permissions.read && (info.permissions.write || info.permissions.execute) {
            return Err(Error::Unsupported(
                "hardware execute/write-only page representation",
            ));
        }
        Ok(u32::from(info.permissions.read)
            | (u32::from(info.writable) << 1)
            | (u32::from(info.permissions.execute) << 2))
    }
    fn pte(&self, va: u64, gpa: u64, flags: u32) -> Result<(), Error> {
        check(unsafe { trames_hvf_page(self.host.as_ptr(), va, gpa, flags) })
    }
    /// Retire stopped CPU aliases before a joined mapping change. Canonical
    /// page owners retain their data; a surviving fork remains independent.
    pub fn synchronize(&mut self, memory: &Memory) -> Result<(), Error> {
        if !memory.parallel_views_quiescent() {
            return Err(Error::Unsupported("active execution Views"));
        }
        let addresses: Vec<_> = self.pages.keys().copied().collect();
        for va in addresses {
            self.pte(va, 0, 0)?;
            self.pages.remove(&va);
        }
        Ok(())
    }
    fn synchronize_view(&mut self, memory: &View) -> Result<(), Error> {
        let retired: Vec<_> = self
            .pages
            .iter()
            .filter_map(|(&va, p)| {
                (p.storage.strong_count() == 0
                    || memory.hardware_page_info(va).ok() != Some(p.info))
                .then_some(va)
            })
            .collect();
        for va in retired {
            self.pte(va, 0, 0)?;
            self.pages.remove(&va);
        }
        for (&va, page) in &self.pages {
            if page.info.writable {
                memory.mark_hardware_write(va)?;
            }
        }
        Ok(())
    }
    fn fault(&mut self, memory: &View, event: &HostExit) -> Result<(), Error> {
        let ec = event.syndrome >> 26;
        let status = event.syndrome & 63;
        if status == 0x21 {
            return Err(MemoryError::Alignment(event.address).into());
        }
        // Cache-maintenance aborts can set WnR even on RX memory (IC IVAU).
        // As in Linux arm64, CM excludes these from ordinary write admission.
        let access = if ec == 0x20 {
            Access::Execute
        } else if event.syndrome & 64 != 0 && event.syndrome & 256 == 0 {
            Access::Write
        } else {
            Access::Read
        };
        if !(4..=15).contains(&status) {
            return Err(Error::Fault {
                pc: event.pc,
                address: event.address,
                syndrome: event.syndrome,
            });
        }
        memory.check(event.address, 1, access)?;
        let va = (event.address & 0x00ff_ffff_ffff_ffff) & !(PAGE_SIZE - 1);
        if (VECTOR_VA..VECTOR_VA + 65536).contains(&va) {
            return Err(Error::Unsupported("reserved exception adapter address"));
        }
        // SAFETY: the scheduler admits every View in hardware mode and joins
        // before coordinator mutation. Canonical exports gate all host borrows.
        let mut allocation_error = None;
        let exported = unsafe {
            memory.export_hardware_page(va, access, |bytes| {
                self.machine.allocate(bytes).map_err(|error| {
                    allocation_error = Some(error);
                    MemoryError::BackingIo(5)
                })
            })
        };
        let (storage, info) =
            exported.map_err(|error| allocation_error.unwrap_or(Error::Memory(error)))?;
        let flags = Self::flags(info)?;
        if info.permissions.execute {
            self.machine.publish_code(storage.physical_address())?;
        }
        self.pte(va, storage.physical_address(), flags)?;
        self.pages.insert(
            va,
            Page {
                gpa: storage.physical_address(),
                storage: Arc::downgrade(&storage),
                info,
            },
        );
        Ok(())
    }
    pub fn run(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut Memory,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        memory.prepare_hardware()?;
        let mut view = memory
            .parallel_view(&mut self.registry)
            .ok_or(Error::Unsupported("hardware view admission"))?;
        view.enable_hardware();
        let result = self.run_parallel(state, &mut view, quantum);
        drop(view);
        memory.commit_parallel()?;
        result
    }
    pub fn run_parallel(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut View,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        if state.tpidr2_el0 != 0 {
            return Err(Error::Unsupported(
                "SME/TPIDR2 is not advertised by this backend",
            ));
        }
        self.wait = None;
        self.synchronize_view(memory)?;
        let deadline = std::time::Instant::now() + quantum;
        loop {
            let mut event = HostExit::default();
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Ok(ExecutionExit {
                    reason: ExitReason::Yield,
                    instructions: InstructionCount::Unavailable,
                });
            }
            // SAFETY: retained VM/physical owners, creator-thread vCPU, bounded
            // timer and exact C register layout. No Rust slice spans this call.
            check(unsafe {
                trames_hvf_run(
                    self.host.as_ptr(),
                    state,
                    left.as_nanos().min(100_000_000) as u64,
                    &mut event,
                )
            })?;
            match event.kind {
                0 | 1 => {
                    return Ok(ExecutionExit {
                        reason: if event.kind == 1 {
                            ExitReason::Syscall
                        } else {
                            ExitReason::Yield
                        },
                        instructions: InstructionCount::Unavailable,
                    })
                }
                2 => self.fault(memory, &event)?,
                4 => {
                    if let Some(wait) = self.machine.resume(event.address)? {
                        self.wait = Some(wait);
                        return Ok(ExecutionExit {
                            reason: ExitReason::Yield,
                            instructions: InstructionCount::Unavailable,
                        });
                    }
                }
                _ => {
                    return Err(Error::Fault {
                        pc: event.pc,
                        address: event.address,
                        syndrome: event.syndrome,
                    })
                }
            }
        }
    }
}
impl Drop for Executor {
    fn drop(&mut self) {
        self.machine.destroy_cpu(self.host);
    }
}

/// One creator-thread vCPU is reserved for joined single-task execution.
pub fn max_parallel_cpus() -> usize {
    (unsafe { trames_hvf_max_vcpus() } as usize)
        .saturating_sub(1)
        .clamp(1, 32)
}

#[cfg(test)]
#[path = "hardware_tests.rs"]
mod hardware_tests;
