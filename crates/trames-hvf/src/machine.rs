// SPDX-License-Identifier: GPL-3.0-or-later
//! One process VM, sparse pageable physical granules, and scoped host borrows.
use super::*;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Condvar, Mutex, Weak,
};
use trames_memory::{HardwareStorage, HardwareWait, SharedMapping};

unsafe extern "C" {
    fn trames_hvf_pressure_supported() -> i32;
    fn trames_hvf_take_pressure() -> u32;
    fn trames_hvf_vm_create(error: *mut i32) -> *mut c_void;
    fn trames_hvf_vm_destroy(vm: *mut c_void);
    fn trames_hvf_cpu_create(vm: *mut c_void, error: *mut i32) -> *mut c_void;
    fn trames_hvf_cpu_destroy(cpu: *mut c_void);
    fn trames_hvf_reserve(vm: *mut c_void) -> u64;
    fn trames_hvf_release(vm: *mut c_void, gpa: u64);
    fn trames_hvf_protect(gpa: u64, length: usize, flags: u32) -> i32;
}
static MACHINE: Mutex<Option<Weak<Machine>>> = Mutex::new(None);
static MACHINE_CHANGED: Condvar = Condvar::new();
static RESIDENT_LIMIT: AtomicU64 = AtomicU64::new(u64::MAX);
static CLOCK: AtomicU64 = AtomicU64::new(0);
thread_local! {
    // A magazine is a weak hint, never an idle RAM/VM owner. At most the
    // currently useful granule is retained as metadata on each execution thread.
    static LOCAL: std::cell::RefCell<Weak<Physical>> = const { std::cell::RefCell::new(Weak::new()) };
}
const ARENA_LEN: usize = 1 << 20;
/// Optional working-set target, independent of the guest's logical RAM limit.
/// macOS memory pressure can also reclaim live, dirty guest pages below it.
pub fn set_resident_limit(bytes: Option<u64>) {
    RESIDENT_LIMIT.store(bytes.unwrap_or(u64::MAX).max(8 << 20), Ordering::Relaxed);
}
#[derive(Default, Debug, Clone, Copy)]
pub struct PagingStats {
    pub mapped_bytes: u64,
    /// Live private VM storage, including host-granule padding; not disk bytes.
    pub backing_bytes: u64,
    pub page_out_bytes: u64,
    pub page_ins: u64,
}
pub(super) struct Machine {
    pointer: NonNull<c_void>,
    granule: usize,
    backing: AtomicU64,
    blocks: Mutex<BTreeMap<u64, Weak<Physical>>>,
    allocation: Mutex<BTreeMap<u64, Weak<Physical>>>,
    arenas: Mutex<BTreeMap<usize, Weak<Arena>>>,
    imports: Mutex<BTreeMap<usize, Weak<Physical>>>,
    mapped: AtomicU64,
    outs: AtomicU64,
    ins: AtomicU64,
}
// SAFETY: VM-wide mapping APIs permit any host thread. Each vCPU is a separate
// !Send owner. Mutexes protect only allocation/mapping metadata, never a run.
unsafe impl Send for Machine {}
unsafe impl Sync for Machine {}
impl Machine {
    pub fn shared() -> Result<Arc<Self>, Error> {
        let mut global = MACHINE.lock().unwrap();
        while let Some(previous) = global.as_ref() {
            if let Some(vm) = previous.upgrade() {
                return Ok(vm);
            }
            // Strong count can reach zero before Drop acquires this lock.
            // Wait for its explicit teardown acknowledgement before creating
            // the next process-wide VM; never race hv_vm_create with destroy.
            global = MACHINE_CHANGED.wait(global).unwrap();
        }
        let granule = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
        if granule != 4 * PAGE_SIZE as usize {
            return Err(Error::Unsupported(
                "hardware physical allocator requires 16KiB host granules",
            ));
        }
        let mut error = 0;
        let pointer =
            NonNull::new(unsafe { trames_hvf_vm_create(&mut error) }).ok_or(Error::Host(error))?;
        let vm = Arc::new(Self {
            pointer,
            granule: unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize },
            backing: AtomicU64::new(0),
            blocks: Mutex::new(BTreeMap::new()),
            allocation: Mutex::new(BTreeMap::new()),
            arenas: Mutex::new(BTreeMap::new()),
            imports: Mutex::new(BTreeMap::new()),
            mapped: AtomicU64::new(0),
            outs: AtomicU64::new(0),
            ins: AtomicU64::new(0),
        });
        *global = Some(Arc::downgrade(&vm));
        Ok(vm)
    }
    pub fn cpu(&self) -> Result<NonNull<c_void>, Error> {
        let mut error = 0;
        NonNull::new(unsafe { trames_hvf_cpu_create(self.pointer.as_ptr(), &mut error) })
            .ok_or(Error::Host(error))
    }
    pub fn destroy_cpu(&self, cpu: NonNull<c_void>) {
        unsafe {
            trames_hvf_cpu_destroy(cpu.as_ptr());
        }
    }
    pub fn allocate(
        self: &Arc<Self>,
        bytes: Option<&[u8]>,
    ) -> Result<Arc<dyn HardwareStorage>, Error> {
        let local = LOCAL.with(|cache| {
            let block = cache.borrow().upgrade()?;
            // Retained data can outlive an earlier VM. Its storage must never
            // enter the replacement VM's allocator via a thread-local hint.
            if block.machine.as_ptr() != Arc::as_ptr(self) {
                return None;
            }
            block.claim().map(|index| (block, index))
        });
        let (block, index) = if let Some(local) = local {
            local
        } else {
            #[cfg(feature = "diagnostics")]
            let measured = diagnostics::start(diagnostics::Work::AllocateWait);
            let mut allocation = self.allocation.lock().unwrap();
            #[cfg(feature = "diagnostics")]
            diagnostics::end(diagnostics::Work::AllocateWait, measured);
            let available = loop {
                let Some((_, candidate)) = allocation.pop_first() else {
                    break None;
                };
                if let Some(block) = candidate.upgrade() {
                    if let Some(index) = block.claim() {
                        if block.used.load(Ordering::Relaxed) != 15 {
                            allocation.insert(block.gpa, Arc::downgrade(&block));
                        }
                        break Some((block, index));
                    }
                }
            };
            // Pool metadata has finished. Neither host VM mapping nor byte
            // initialization may serialize unrelated allocators under this lock.
            drop(allocation);
            if let Some(available) = available {
                available
            } else {
                let arena = self.allocate_arena()?;
                let ptr = arena.pointer();
                let gpa = unsafe { trames_hvf_reserve(self.pointer.as_ptr()) };
                #[cfg(feature = "diagnostics")]
                let measured = diagnostics::start(diagnostics::Work::Map);
                if let Err(e) = check(unsafe {
                    trames_hvf_map(std::ptr::null_mut(), ptr.cast(), gpa, self.granule)
                }) {
                    unsafe { trames_hvf_release(self.pointer.as_ptr(), gpa) };
                    return Err(e);
                }
                #[cfg(feature = "diagnostics")]
                diagnostics::end(diagnostics::Work::Map, measured);
                self.backing
                    .fetch_add(self.granule as u64, Ordering::Relaxed);
                let block = Arc::new(Physical {
                    pointer: NonNull::new(ptr.cast()).unwrap(),
                    gpa,
                    external: None,
                    _arena: Some(arena),
                    length: self.granule,
                    machine: Arc::downgrade(self),
                    used: AtomicU8::new(1),
                    executable: AtomicU8::new(0),
                    touched: AtomicU64::new(CLOCK.fetch_add(1, Ordering::Relaxed)),
                    gate: Mutex::new(Gate {
                        hosts: 0,
                        mapped: true,
                        owner: None,
                        waiters: Vec::new(),
                    }),
                });
                self.mapped
                    .fetch_add(self.granule as u64, Ordering::Relaxed);
                self.blocks
                    .lock()
                    .unwrap()
                    .insert(gpa, Arc::downgrade(&block));
                self.allocation
                    .lock()
                    .unwrap()
                    .insert(gpa, Arc::downgrade(&block));
                (block, 0)
            }
        };
        LOCAL.with(|cache| *cache.borrow_mut() = Arc::downgrade(&block));
        block
            .touched
            .store(CLOCK.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        let slot = Arc::new(Slot { block, index });
        // SAFETY: newly claimed disjoint 4K slot. No previous page/table owner
        // survives reuse. Other hardware actors can access only other slots.
        #[cfg(feature = "diagnostics")]
        let measured = diagnostics::start(diagnostics::Work::Initialize);
        unsafe {
            if let Some(bytes) = bytes {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), slot.pointer(), PAGE_SIZE as usize);
            } else {
                std::ptr::write_bytes(slot.pointer(), 0, PAGE_SIZE as usize);
            }
        }
        #[cfg(feature = "diagnostics")]
        diagnostics::end(diagnostics::Work::Initialize, measured);
        let limit = RESIDENT_LIMIT.load(Ordering::Relaxed);
        if limit != u64::MAX {
            self.trim_to(limit, slot.block.gpa)?;
        }
        Ok(slot)
    }
    fn allocate_arena(self: &Arc<Self>) -> Result<ArenaSlot, Error> {
        let mut arenas = self.arenas.lock().unwrap();
        let arena = loop {
            match arenas.pop_first() {
                Some((_, candidate)) => {
                    if let Some(arena) = candidate.upgrade() {
                        break arena;
                    }
                }
                None => {
                    // Reserve VA only. HVF maps and wires one used host granule
                    // at a time; untouched arena bytes have no physical owner.
                    let pointer = unsafe {
                        libc::mmap(
                            std::ptr::null_mut(),
                            ARENA_LEN,
                            libc::PROT_READ | libc::PROT_WRITE,
                            libc::MAP_PRIVATE | libc::MAP_ANON,
                            -1,
                            0,
                        )
                    };
                    if pointer == libc::MAP_FAILED {
                        return Err(Error::Io(std::io::Error::last_os_error()));
                    }
                    break Arc::new(Arena {
                        pointer: NonNull::new(pointer.cast()).unwrap(),
                        free: AtomicU64::new(u64::MAX),
                        reusable: AtomicU64::new(0),
                    });
                }
            }
        };
        let free = arena.free.load(Ordering::Relaxed);
        assert_ne!(free, 0);
        let index = free.trailing_zeros() as usize;
        let bit = 1 << index;
        arena.free.store(free & !bit, Ordering::Relaxed);
        if arena.reusable.fetch_and(!bit, Ordering::Relaxed) & bit != 0 {
            // SAFETY: the arena lock owns this entire unused host granule.
            // End its reusable state before publishing any new HVF/byte owner.
            assert_eq!(
                unsafe {
                    libc::madvise(
                        arena.pointer.as_ptr().add(index * self.granule).cast(),
                        self.granule,
                        libc::MADV_FREE_REUSE,
                    )
                },
                0,
                "reactivate anonymous RAM before mapping a granule"
            );
        }
        if free & !bit != 0 {
            arenas.insert(arena.pointer.as_ptr() as usize, Arc::downgrade(&arena));
        }
        Ok(ArenaSlot {
            arena,
            index,
            machine: Arc::downgrade(self),
        })
    }
    pub fn import(
        self: &Arc<Self>,
        mapping: SharedMapping,
    ) -> Result<Arc<dyn HardwareStorage>, Error> {
        let address = mapping.pointer() as usize;
        let base = address & !(self.granule - 1);
        let allocation = mapping.allocation_pointer() as usize;
        if !address.is_multiple_of(PAGE_SIZE as usize)
            || base < allocation
            || base
                .checked_add(self.granule)
                .is_none_or(|end| end > allocation.saturating_add(mapping.allocation_len()))
        {
            return Err(Error::Unsupported(
                "shared mapping has no complete host granule",
            ));
        }
        let mut imports = self.imports.lock().unwrap();
        let block = if let Some(block) = imports.get(&base).and_then(Weak::upgrade) {
            block
        } else {
            imports.retain(|_, p| p.strong_count() != 0);
            let gpa = unsafe { trames_hvf_reserve(self.pointer.as_ptr()) };
            if let Err(error) = check(unsafe {
                trames_hvf_map(std::ptr::null_mut(), base as *mut c_void, gpa, self.granule)
            }) {
                unsafe {
                    trames_hvf_release(self.pointer.as_ptr(), gpa);
                }
                return Err(error);
            }
            // This original file mapping stays put. Display/GPU readers use raw
            // pointers under protocol leases; Rust CPU borrows use this gate.
            let block = Arc::new(Physical {
                pointer: NonNull::new(base as *mut u8).unwrap(),
                gpa,
                external: Some(mapping),
                _arena: None,
                length: self.granule,
                machine: Arc::downgrade(self),
                used: AtomicU8::new(15),
                executable: AtomicU8::new(0),
                touched: AtomicU64::new(CLOCK.fetch_add(1, Ordering::Relaxed)),
                gate: Mutex::new(Gate {
                    hosts: 0,
                    mapped: true,
                    owner: None,
                    waiters: Vec::new(),
                }),
            });
            self.mapped
                .fetch_add(self.granule as u64, Ordering::Relaxed);
            self.blocks
                .lock()
                .unwrap()
                .insert(gpa, Arc::downgrade(&block));
            imports.insert(base, Arc::downgrade(&block));
            block
        };
        let index = (address - base) / PAGE_SIZE as usize;
        drop(imports);
        let slot = Arc::new(Slot { block, index });
        let limit = RESIDENT_LIMIT.load(Ordering::Relaxed);
        if limit != u64::MAX {
            self.trim_to(limit, slot.block.gpa)?;
        }
        Ok(slot)
    }
    pub fn publish_code(&self, address: u64) -> Result<(), Error> {
        let gpa = address & !(self.granule as u64 - 1);
        let block = self
            .blocks
            .lock()
            .unwrap()
            .get(&gpa)
            .and_then(Weak::upgrade)
            .ok_or(Error::Unsupported("unowned executable page"))?;
        let index = (address - gpa) as usize / PAGE_SIZE as usize;
        let _gate = block.gate.lock().unwrap();
        block.executable.fetch_or(1 << index, Ordering::Relaxed);
        // Cache operations do not construct a byte reference. Initial code and
        // later page-in publish only actual executable slots, not untouched RAM.
        unsafe {
            trames_hvf_icache(
                block
                    .pointer
                    .as_ptr()
                    .add(index * PAGE_SIZE as usize)
                    .cast(),
                PAGE_SIZE as usize,
            );
        }
        Ok(())
    }
    pub fn resume(&self, address: u64) -> Result<Option<Arc<HardwareWait>>, Error> {
        let gpa = address & !(self.granule as u64 - 1);
        let block = self
            .blocks
            .lock()
            .unwrap()
            .get(&gpa)
            .and_then(Weak::upgrade)
            .ok_or(Error::Unsupported("unowned stage-two fault"))?;
        let mut gate = block.gate.lock().unwrap();
        block
            .touched
            .store(CLOCK.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
        if gate.hosts != 0 {
            let wait = Arc::new(HardwareWait::default());
            gate.waiters.push(wait.clone());
            return Ok(Some(wait));
        }
        if !gate.mapped {
            let code = block.executable.load(Ordering::Relaxed);
            for index in 0..4 {
                if code & (1 << index) != 0 {
                    unsafe {
                        trames_hvf_icache(
                            block
                                .pointer
                                .as_ptr()
                                .add(index * PAGE_SIZE as usize)
                                .cast(),
                            PAGE_SIZE as usize,
                        );
                    }
                }
            }
            check(unsafe {
                trames_hvf_map(
                    std::ptr::null_mut(),
                    block.pointer.as_ptr().cast(),
                    block.gpa,
                    block.length,
                )
            })?;
            gate.mapped = true;
            self.mapped
                .fetch_add(block.length as u64, Ordering::Relaxed);
            self.ins.fetch_add(1, Ordering::Relaxed);
        }
        drop(gate);
        let limit = RESIDENT_LIMIT.load(Ordering::Relaxed);
        if limit != u64::MAX {
            self.trim_to(limit, block.gpa)?;
        }
        Ok(None)
    }
    fn blocks(&self) -> Vec<Arc<Physical>> {
        let mut blocks = self.blocks.lock().unwrap();
        blocks.retain(|_, p| p.strong_count() != 0);
        blocks.values().filter_map(Weak::upgrade).collect()
    }
    pub fn stats(&self) -> PagingStats {
        PagingStats {
            mapped_bytes: self.mapped.load(Ordering::Relaxed),
            backing_bytes: self.backing.load(Ordering::Relaxed),
            page_out_bytes: self.outs.load(Ordering::Relaxed),
            page_ins: self.ins.load(Ordering::Relaxed),
        }
    }
    fn trim_to(&self, target: u64, avoid: u64) -> Result<u64, Error> {
        let mut mapped = self.mapped.load(Ordering::Relaxed);
        if mapped <= target {
            return Ok(0);
        }
        let target = target.saturating_sub(1 << 20);
        let mut blocks = self.blocks();
        blocks.sort_unstable_by_key(|b| b.touched.load(Ordering::Relaxed));
        let mut reclaimed = 0;
        for block in blocks {
            if mapped <= target {
                break;
            }
            if block.gpa == avoid {
                continue;
            }
            let mut gate = block.gate.lock().unwrap();
            if !gate.mapped || gate.hosts != 0 {
                continue;
            }
            // hv_vm_unmap completes stage-two invalidation before any eviction.
            // The stable host VM object retains bytes. Once HVF releases the
            // mapping, macOS can compress/page it through its native pager.
            // Never discard anonymous contents or perform synchronous disk I/O.
            check(unsafe { trames_hvf_unmap(std::ptr::null_mut(), block.gpa, block.length) })?;
            gate.mapped = false;
            // Darwin DONTNEED uses VM_SYNC_DEACTIVATE, not KILLPAGES: retain
            // every byte and let the native pager reclaim inactive residency.
            // See apple-oss-distributions/xnu/osfmk/vm/vm_map.c. This file is
            // macOS-only; Linux's destructive DONTNEED semantics do not apply.
            unsafe {
                libc::madvise(
                    block.pointer.as_ptr().cast(),
                    block.length,
                    libc::MADV_DONTNEED,
                );
            }
            self.mapped
                .fetch_sub(block.length as u64, Ordering::Relaxed);
            mapped -= block.length as u64;
            reclaimed += block.length as u64;
        }
        self.outs.fetch_add(reclaimed, Ordering::Relaxed);
        Ok(reclaimed)
    }
}
impl Drop for Machine {
    fn drop(&mut self) {
        // All thread-affine vCPUs have already been destroyed. Physical page
        // owners may outlive the VM; their weak VM handle then grants no access.
        let mut global = MACHINE.lock().unwrap();
        unsafe {
            trames_hvf_vm_destroy(self.pointer.as_ptr());
        }
        *global = None;
        MACHINE_CHANGED.notify_all();
    }
}
// The pool retains weak handles only. The last physical owner unmaps its
// complete arena; freed groups in a partly live arena return to the OS pager.
struct Arena {
    pointer: NonNull<u8>,
    free: AtomicU64,
    reusable: AtomicU64,
}
// SAFETY: immutable VA ownership. Only the arena pool lock edits group metadata;
// actual byte access uses disjoint Physical owners and their existing gates.
unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}
impl Drop for Arena {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.pointer.as_ptr().cast(), ARENA_LEN) };
    }
}
struct ArenaSlot {
    arena: Arc<Arena>,
    index: usize,
    machine: Weak<Machine>,
}
impl ArenaSlot {
    fn pointer(&self) -> *mut u8 {
        self.arena
            .pointer
            .as_ptr()
            .wrapping_add(self.index * 4 * PAGE_SIZE as usize)
    }
}
impl Drop for ArenaSlot {
    fn drop(&mut self) {
        let Some(machine) = self.machine.upgrade() else {
            return;
        };
        #[cfg(feature = "diagnostics")]
        let measured = diagnostics::start(diagnostics::Work::Recycle);
        let mut arenas = machine.arenas.lock().unwrap();
        // Physical::drop has withdrawn HVF before this field is retired. There
        // are no surviving byte owners in the full aligned 16KiB group.
        if unsafe {
            libc::madvise(
                self.pointer().cast(),
                machine.granule,
                libc::MADV_FREE_REUSABLE,
            )
        } == 0
        {
            self.arena
                .reusable
                .fetch_or(1 << self.index, Ordering::Relaxed);
        }
        self.arena.free.fetch_or(1 << self.index, Ordering::Relaxed);
        arenas.insert(
            self.arena.pointer.as_ptr() as usize,
            Arc::downgrade(&self.arena),
        );
        #[cfg(feature = "diagnostics")]
        diagnostics::end(diagnostics::Work::Recycle, measured);
    }
}
struct Gate {
    hosts: usize,
    mapped: bool,
    owner: Option<Arc<Machine>>,
    waiters: Vec<Arc<HardwareWait>>,
}
struct Physical {
    pointer: NonNull<u8>,
    gpa: u64,
    external: Option<SharedMapping>,
    _arena: Option<ArenaSlot>,
    length: usize,
    machine: Weak<Machine>,
    used: AtomicU8,
    // Monotonic: former code slots still publish on reuse/page-in.
    executable: AtomicU8,
    touched: AtomicU64,
    gate: Mutex<Gate>,
}
impl Physical {
    fn claim(&self) -> Option<usize> {
        // Only slot ownership, not guest data ordering. Release on the final
        // previous Slot pairs with Acquire before zero/copy and publication.
        // Local and shared-pool candidates can race; only the CAS winner owns
        // that 4KiB range. A full granule returns immediately, never waits.
        self.used
            .try_update(Ordering::Acquire, Ordering::Relaxed, |used| {
                let free = !used & 15;
                (free != 0).then(|| used | (1 << free.trailing_zeros()))
            })
            .ok()
            .map(|used| (!used & 15).trailing_zeros() as usize)
    }
}
unsafe impl Send for Physical {}
unsafe impl Sync for Physical {}
impl Drop for Physical {
    fn drop(&mut self) {
        if let Some(vm) = self.machine.upgrade() {
            if self.gate.get_mut().unwrap().mapped {
                #[cfg(feature = "diagnostics")]
                let measured = diagnostics::start(diagnostics::Work::Unmap);
                unsafe {
                    trames_hvf_unmap(std::ptr::null_mut(), self.gpa, self.length);
                }
                #[cfg(feature = "diagnostics")]
                diagnostics::end(diagnostics::Work::Unmap, measured);
                vm.mapped.fetch_sub(self.length as u64, Ordering::Relaxed);
            }
            unsafe {
                trames_hvf_release(vm.pointer.as_ptr(), self.gpa);
            }
            if self.external.is_none() {
                vm.backing.fetch_sub(self.length as u64, Ordering::Relaxed);
            }
        }
    }
}
struct Slot {
    block: Arc<Physical>,
    index: usize,
}
// SAFETY: one retained stable 4K slot. Stage-two protection gates all hardware
// aliases of the containing host granule before Rust references are exposed.
// The original 64-byte locks still arbitrate overlapping host byte borrows.
unsafe impl HardwareStorage for Slot {
    fn pointer(&self) -> *mut u8 {
        self.block
            .pointer
            .as_ptr()
            .wrapping_add(self.index * PAGE_SIZE as usize)
    }
    fn physical_address(&self) -> u64 {
        self.block.gpa + (self.index as u64 * PAGE_SIZE)
    }
    fn is_live(&self) -> bool {
        self.block.machine.strong_count() != 0
    }
    fn shared_mapping(&self) -> Option<SharedMapping> {
        let mapping = self.block.external.as_ref()?;
        mapping.with_offset(self.pointer() as usize - mapping.allocation_pointer() as usize)
    }
    fn publish_host_write(&self, offset: usize, len: usize) {
        if len != 0 && self.block.executable.load(Ordering::Relaxed) & (1 << self.index) != 0 {
            // The owning slot keeps this initialized host VA valid even while
            // stage two is evicted. Actual code writers publish their range;
            // consumers never flush every executable page on a global epoch.
            unsafe {
                trames_hvf_icache(self.pointer().add(offset).cast(), len);
            }
        }
    }
    fn begin_host_access(&self) {
        let mut gate = self.block.gate.lock().unwrap();
        if gate.hosts == 0 {
            gate.owner = self.block.machine.upgrade();
        }
        if gate.hosts == 0 && gate.mapped && gate.owner.is_some() {
            assert_eq!(
                unsafe { trames_hvf_protect(self.block.gpa, self.block.length, 0) },
                0,
                "failed to exclude hardware byte actors"
            );
        }
        gate.hosts += 1;
    }
    fn end_host_access(&self) {
        let mut gate = self.block.gate.lock().unwrap();
        gate.hosts -= 1;
        if gate.hosts != 0 {
            return;
        }
        if gate.mapped && gate.owner.is_some() {
            assert_eq!(
                unsafe { trames_hvf_protect(self.block.gpa, self.block.length, 7) },
                0,
                "failed to restore hardware byte actors"
            );
        }
        let waiters = std::mem::take(&mut gate.waiters);
        let owner = gate.owner.take();
        drop(gate);
        for wait in waiters {
            wait.fire();
        }
        drop(owner);
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        if self.block.external.is_some() {
            return;
        }
        let previous = self
            .block
            .used
            .fetch_and(!(1 << self.index), Ordering::Release);
        if previous == 15 {
            if let Some(vm) = self.block.machine.upgrade() {
                vm.allocation
                    .lock()
                    .unwrap()
                    .insert(self.block.gpa, Arc::downgrade(&self.block));
            }
        }
    }
}

pub fn paging_stats() -> PagingStats {
    let machine = { MACHINE.lock().unwrap().as_ref().and_then(Weak::upgrade) };
    machine.map_or(PagingStats::default(), |m| m.stats())
}
/// Reclaim up to the requested live physical bytes, preserving all contents.
pub fn reclaim(bytes: u64) -> Result<u64, Error> {
    let machine = MACHINE.lock().unwrap().as_ref().and_then(Weak::upgrade);
    let Some(machine) = machine else {
        return Ok(0);
    };
    machine.trim_to(machine.stats().mapped_bytes.saturating_sub(bytes), u64::MAX)
}
/// Latch-only OS event subscription; no guest-byte access or eviction here.
/// Returns 0 for no update, 1 normal, 2 warning, or 4 critical pressure.
pub fn take_memory_pressure() -> Result<u32, Error> {
    if unsafe { trames_hvf_pressure_supported() } == 0 {
        return Err(Error::Unsupported(
            "macOS memory-pressure notifications unavailable",
        ));
    }
    Ok(unsafe { trames_hvf_take_pressure() })
}
