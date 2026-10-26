// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    collections::BTreeMap,
    ffi::c_void,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{Arc, Weak},
    time::Duration,
};
use trames_execution::{Aarch64State, ExecutionExit, ExitReason, Features, InstructionCount};
use trames_memory::{Access, HardwarePageInfo, Memory, MemoryError, SharedPage, PAGE_SIZE};

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
    fn trames_hvf_create(error: *mut i32) -> *mut c_void;
    fn trames_hvf_destroy(host: *mut c_void);
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

struct Physical {
    pointer: NonNull<u8>,
    length: usize,
}
// SAFETY: immutable allocation ownership, not byte authority. SharedPage guards
// protect host access; the one-thread executor excludes byte actors while live.
unsafe impl Send for Physical {}
unsafe impl Sync for Physical {}
impl Physical {
    fn new(length: usize) -> Result<Arc<Self>, Error> {
        // SAFETY: private, non-executable anonymous host storage, OS-selected VA.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            return Err(Error::Host(-1));
        }
        let Some(pointer) = NonNull::new(p.cast()) else {
            return Err(Error::Host(-1));
        };
        Ok(Arc::new(Self { pointer, length }))
    }
}
impl Drop for Physical {
    fn drop(&mut self) {
        // SAFETY: final physical owner, after every hardware mapping/byte owner ends.
        unsafe {
            libc::munmap(self.pointer.as_ptr().cast(), self.length);
        }
    }
}
struct Page {
    bytes: Weak<SharedPage>,
    info: HardwarePageInfo,
}
struct Chunk {
    physical: Arc<Physical>,
    gpa: u64,
    pages: BTreeMap<u64, Page>,
    used: u64,
}

/// One hardware VM with one thread-affine vCPU. No Linux kernel or boot RAM.
/// Multi-task scheduling/signals are a separate common-runtime integration gate.
pub struct Executor {
    host: NonNull<c_void>,
    chunks: BTreeMap<u64, Chunk>,
    next_gpa: u64,
    free_gpas: Vec<u64>,
    granule: usize,
    _same_thread: PhantomData<Rc<()>>,
}
impl Executor {
    pub fn new() -> Result<Self, Error> {
        assert_eq!(std::mem::size_of::<Aarch64State>(), 816);
        let mut error = 0;
        // SAFETY: complete error output; C owns all partial-initialization cleanup.
        let host =
            NonNull::new(unsafe { trames_hvf_create(&mut error) }).ok_or(Error::Host(error))?;
        // SAFETY: sysconf has no pointers; supported Darwin host page size.
        let granule = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
        Ok(Self {
            host,
            chunks: BTreeMap::new(),
            next_gpa: 0x100000000,
            free_gpas: Vec::new(),
            granule,
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
        self.chunks.len() * self.granule
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
        // SAFETY: stopped vCPU, owned page tables, no guest virtual host pointers.
        check(unsafe { trames_hvf_page(self.host.as_ptr(), va, gpa, flags) })
    }
    fn detach_chunk(&mut self, base: u64) -> Result<(), Error> {
        let Some(chunk) = self.chunks.get(&base) else {
            return Ok(());
        };
        for &va in chunk.pages.keys() {
            self.pte(va, 0, 0)?;
        }
        // SAFETY: stopped single vCPU; old byte owners (e.g. fork) keep physical
        // data, while this parent's new hardware binding gets separate storage.
        check(unsafe { trames_hvf_unmap(self.host.as_ptr(), chunk.gpa, self.granule) })?;
        self.free_gpas.push(chunk.gpa);
        self.chunks.remove(&base);
        Ok(())
    }
    /// Retire hardware aliases before their physical owners. Called after every
    /// joined syscall, including discard, heap shrink, replacement and exit.
    pub fn synchronize(&mut self, memory: &Memory) -> Result<(), Error> {
        if !memory.parallel_views_quiescent() {
            return Err(Error::Unsupported("active execution Views"));
        }
        let mut changes = Vec::new();
        for (&base, chunk) in &self.chunks {
            for (&va, page) in &chunk.pages {
                match memory.hardware_page_info(va) {
                    Ok(info)
                        if info.storage_id == page.info.storage_id
                            && info.backing_version == page.info.backing_version
                            && page.bytes.strong_count() != 0 =>
                    {
                        if info != page.info {
                            changes.push((base, va, Some(info)));
                        }
                    }
                    _ => changes.push((base, va, None)),
                }
            }
        }
        for (base, va, info) in changes {
            if let Some(info) = info {
                let chunk = &self.chunks[&base];
                let offset = (va - base) as usize;
                if info.permissions.execute {
                    // SAFETY: one complete live page, vCPU stopped; publish data
                    // writes before permitting hardware instruction fetch.
                    unsafe {
                        trames_hvf_icache(
                            chunk.physical.pointer.as_ptr().add(offset).cast(),
                            PAGE_SIZE as usize,
                        );
                    }
                }
                self.pte(va, chunk.gpa + va - base, Self::flags(info)?)?;
                self.chunks
                    .get_mut(&base)
                    .unwrap()
                    .pages
                    .get_mut(&va)
                    .unwrap()
                    .info = info;
            } else {
                self.pte(va, 0, 0)?;
                self.chunks.get_mut(&base).unwrap().pages.remove(&va);
            }
        }
        let empty: Vec<_> = self
            .chunks
            .iter()
            .filter_map(|(&base, c)| c.pages.is_empty().then_some(base))
            .collect();
        for base in empty {
            let chunk = &self.chunks[&base];
            // SAFETY: vCPU stopped, all stage-one aliases removed before stage
            // two unmap and before dropping this physical mapping's owner.
            check(unsafe { trames_hvf_unmap(self.host.as_ptr(), chunk.gpa, self.granule) })?;
            self.free_gpas.push(chunk.gpa);
            self.chunks.remove(&base);
        }
        Ok(())
    }
    fn fault(&mut self, memory: &mut Memory, event: &HostExit) -> Result<(), Error> {
        let ec = event.syndrome >> 26;
        let access = if ec == 0x20 {
            Access::Execute
        } else if event.syndrome & 64 != 0 {
            Access::Write
        } else {
            Access::Read
        };
        memory.check(event.address, 1, access)?;
        let va = (event.address & 0x00ff_ffff_ffff_ffff) & !(PAGE_SIZE - 1);
        if (VECTOR_VA..VECTOR_VA + self.granule as u64).contains(&va) {
            return Err(Error::Unsupported("reserved exception adapter address"));
        }
        let info = memory.hardware_page_info(va)?;
        let base = va & !(self.granule as u64 - 1);
        if access == Access::Write && !info.exclusive {
            self.detach_chunk(base)?;
        }
        if !self.chunks.contains_key(&base) {
            let physical = Physical::new(self.granule)?;
            let gpa = self.free_gpas.pop().unwrap_or_else(|| {
                let gpa = self.next_gpa;
                self.next_gpa += self.granule as u64;
                gpa
            });
            // SAFETY: aligned retained allocation with exactly one owned GPA.
            check(unsafe {
                trames_hvf_map(
                    self.host.as_ptr(),
                    physical.pointer.as_ptr().cast(),
                    gpa,
                    self.granule,
                )
            })?;
            self.chunks.insert(
                base,
                Chunk {
                    physical,
                    gpa,
                    pages: BTreeMap::new(),
                    used: 0,
                },
            );
        }
        let chunk = &self.chunks[&base];
        let offset = (va - base) as usize;
        let storage = chunk
            .pages
            .get(&va)
            .and_then(|p| p.bytes.upgrade())
            .unwrap_or_else(|| {
                // SAFETY: this exact complete page is retained by its immutable
                // physical owner. All host bytes use SharedPage guards; hardware
                // executes only under this thread's exclusive Memory borrow.
                Arc::new(unsafe {
                    SharedPage::from_mapping(
                        chunk.physical.pointer.as_ptr().add(offset),
                        chunk.physical.clone(),
                    )
                })
            });
        // SAFETY: this backend is single-threaded; no concurrent View/pin may
        // execute. No Rust byte slice exists across trames_hvf_run.
        let fresh = chunk.used & (1 << (offset / PAGE_SIZE as usize)) == 0;
        let info = unsafe {
            memory.bind_hardware_page(va, storage.clone(), access == Access::Write, fresh)
        }?;
        let chunk = &self.chunks[&base];
        if info.permissions.execute {
            // SAFETY: initialized complete code page, still stopped.
            unsafe {
                trames_hvf_icache(
                    chunk.physical.pointer.as_ptr().add(offset).cast(),
                    PAGE_SIZE as usize,
                );
            }
        }
        self.pte(va, chunk.gpa + va - base, Self::flags(info)?)?;
        self.chunks.get_mut(&base).unwrap().pages.insert(
            va,
            Page {
                bytes: Arc::downgrade(&storage),
                info,
            },
        );
        self.chunks.get_mut(&base).unwrap().used |= 1 << (offset / PAGE_SIZE as usize);
        Ok(())
    }
    pub fn run(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut Memory,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        if state.tpidr2_el0 != 0 {
            return Err(Error::Unsupported(
                "SME/TPIDR2 is not advertised by this backend",
            ));
        }
        self.synchronize(memory)?;
        let result = self.execute(state, memory, quantum);
        // Publish after hardware stops, before the host kernel can access
        // guest bytes or invalidate exclusive/code state. No ordinary guest
        // instruction acquires this guard or increments a Rust counter.
        for chunk in self.chunks.values() {
            for (&va, page) in &chunk.pages {
                if page.info.writable {
                    // SAFETY: the only hardware CPU is stopped and its Memory
                    // is exclusively borrowed; original page bindings live.
                    unsafe { memory.finish_hardware_write(va) }?;
                }
            }
        }
        result
    }
    fn execute(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut Memory,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
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
            // SAFETY: exact repr(C) state/layout, retained host VM, creator
            // thread, exclusive guest memory ownership, bounded hardware timer.
            check(unsafe {
                trames_hvf_run(
                    self.host.as_ptr(),
                    state,
                    left.as_nanos().min(100_000_000) as u64,
                    &mut event,
                )
            })?;
            match event.kind {
                0 => {
                    return Ok(ExecutionExit {
                        reason: ExitReason::Yield,
                        instructions: InstructionCount::Unavailable,
                    })
                }
                1 => {
                    return Ok(ExecutionExit {
                        reason: ExitReason::Syscall,
                        instructions: InstructionCount::Unavailable,
                    })
                }
                2 => self.fault(memory, &event)?,
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
        // SAFETY: creator-thread destruction first removes the vCPU and entire VM;
        // only afterward may Rust drop GPA owners. Memory retains its own bytes.
        unsafe {
            trames_hvf_destroy(self.host.as_ptr());
        }
    }
}
