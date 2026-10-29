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
use trames_execution::{
    Aarch64State, ExecutionExit, ExitReason, Features, InstructionCount, SmeState,
};
use trames_memory::{
    Access, CpuMemory, HardwarePageInfo, HardwareStorage, HardwareWait, Memory, MemoryError,
    PAGE_SIZE,
    parallel::{Registry, View},
};
#[cfg(feature = "diagnostics")]
#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "machine.rs"]
mod machine;
use machine::Machine;
pub use machine::{PagingStats, paging_stats, reclaim, set_resident_limit, take_memory_pressure};
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
    fn trames_hvf_features(hwcap: *mut u64, hwcap2: *mut u64);
    fn trames_hvf_sme_input(host: *mut c_void, state: *const SmeState, vl: u64) -> i32;
    fn trames_hvf_sme_active(host: *mut c_void) -> u64;
    fn trames_hvf_sme_output(host: *mut c_void, state: *mut SmeState);

    fn trames_hvf_register_stats(host: *mut c_void, stats: *mut RegisterStats);
    fn trames_hvf_max_vcpus() -> u32;
    fn trames_hvf_space_current(host: *mut c_void) -> *mut c_void;
    fn trames_hvf_space_create(host: *mut c_void, error: *mut i32) -> *mut c_void;
    fn trames_hvf_space_select(host: *mut c_void, space: *mut c_void) -> i32;
    fn trames_hvf_space_destroy(host: *mut c_void, space: *mut c_void) -> i32;
    fn trames_hvf_page(host: *mut c_void, va: u64, gpa: u64, flags: u32) -> i32;
    fn trames_hvf_block(host: *mut c_void, va: u64, gpa: u64, flags: u32) -> i32;
    fn trames_hvf_map(host: *mut c_void, memory: *mut c_void, gpa: u64, size: usize) -> i32;
    fn trames_hvf_unmap(host: *mut c_void, gpa: u64, size: usize) -> i32;
    fn trames_hvf_icache(memory: *mut c_void, size: usize);
    fn trames_hvf_run(
        host: *mut c_void,
        state: *mut Aarch64State,
        nanos: u64,
        continue_fault: bool,
        result: *mut HostExit,
    ) -> i32;
}
pub fn features() -> Features {
    static FEATURES: std::sync::OnceLock<Features> = std::sync::OnceLock::new();
    *FEATURES.get_or_init(|| {
        let mut result = Features {
            hwcap: 3,
            hwcap2: 0,
        };
        // SAFETY: the C helper writes two local u64s using HVF's configuration
        // query and never retains the pointers or creates a running vCPU.
        unsafe { trames_hvf_features(&mut result.hwcap, &mut result.hwcap2) };
        result
    })
}
pub fn feature_names() -> &'static str {
    static NAMES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let feature = features();
        let mut names = vec!["fp", "asimd"];
        for (bit, name) in [
            (3, "aes"),
            (4, "pmull"),
            (5, "sha1"),
            (6, "sha2"),
            (7, "crc32"),
            (8, "atomics"),
            (9, "fphp"),
            (10, "asimdhp"),
            (12, "asimdrdm"),
            (20, "asimddp"),
            (23, "asimdfhm"),
        ] {
            if feature.hwcap & (1 << bit) != 0 {
                names.push(name);
            }
        }
        for (bit, name) in [
            (13, "i8mm"),
            (14, "bf16"),
            (23, "sme"),
            (24, "sme_i16i64"),
            (25, "sme_f64f64"),
            (26, "sme_i8i32"),
            (27, "sme_f16f32"),
            (28, "sme_b16f32"),
            (29, "sme_f32f32"),
            (37, "sme2"),
        ] {
            if feature.hwcap2 & (1 << bit) != 0 {
                names.push(name);
            }
        }
        names.join(" ")
    })
}
/// Thread-local transport counts, read only on the owning executor thread.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisterStats {
    pub mailbox: u64,
    pub fallback: u64,
    pub completions: u64,
    pub entries: u64,
    pub invalidations: u64,
    pub full_invalidations: u64,
    pub restore_interruptions: u64,
    pub fault_continuations: u64,
    pub timer_programs: u64,
    pub publications: u64,
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
    write: Option<trames_memory::HardwareWriteFrame>,
}
// Bound inactive metadata without retaining guest RAM. Each worker keeps one
// ASID, flushing it on root changes; cached roots never consume extra ASIDs.
const CACHED_SPACES: usize = 8;
const CACHED_PAGES: usize = 131_072;
struct CachedChunk {
    pages: usize,
    version: Option<u64>,
}
struct Block {
    storage: Weak<dyn trames_memory::HardwareRange>,
    version: Option<u64>,
}
struct AddressSpace {
    blocks: BTreeMap<u64, Block>,
    host: NonNull<c_void>,
    pages: BTreeMap<u64, Page>,
    chunks: BTreeMap<u64, CachedChunk>,
    version: Option<(u64, u64)>,
    used: u64,
    publications: BTreeMap<u64, trames_memory::HardwareWriteFrame>,
    private_writable: usize,
}
impl AddressSpace {
    fn remove_page(&mut self, va: u64) -> Option<Page> {
        let page = self.pages.remove(&va)?;
        let key = va / trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
        let chunk = self.chunks.get_mut(&key).expect("cached page chunk");
        chunk.pages -= 1;
        if chunk.pages == 0 {
            self.chunks.remove(&key);
        }
        Some(page)
    }
    fn track_write(
        &mut self,
        va: u64,
        old: Option<trames_memory::HardwareWriteFrame>,
        new: Option<trames_memory::HardwareWriteFrame>,
    ) {
        self.private_writable -= usize::from(old.is_some_and(|write| !write.needs_publication()));
        self.private_writable += usize::from(new.is_some_and(|write| !write.needs_publication()));
        if let Some(write) = new.filter(|write| write.needs_publication()) {
            self.publications.insert(va, write);
        } else {
            self.publications.remove(&va);
        }
    }
}
#[cfg(test)]
#[derive(Default, Debug, Clone, Copy)]
struct TranslationStats {
    faults: usize,
    inspections: usize,
    exports: usize,
    updates: usize,
    retired: usize,
    blocks: usize,
}
/// A thread-affine vCPU with persistent address-space tables in the shared VM.
/// Canonical physical pages, VFS and native access share one byte authority.
pub struct Executor {
    host: NonNull<c_void>,
    machine: Arc<Machine>,
    spaces: BTreeMap<u64, AddressSpace>,
    active: u64,
    clock: u64,
    #[cfg(test)]
    translations: TranslationStats,
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
            spaces: BTreeMap::from([(
                0,
                AddressSpace {
                    host: NonNull::new(unsafe { trames_hvf_space_current(host.as_ptr()) })
                        .expect("initial tables"),
                    blocks: BTreeMap::new(),
                    pages: BTreeMap::new(),
                    chunks: BTreeMap::new(),
                    version: None,
                    used: 0,
                    publications: BTreeMap::new(),
                    private_writable: 0,
                },
            )]),
            active: 0,
            clock: 0,
            #[cfg(test)]
            translations: TranslationStats::default(),
            registry,
            wait: None,
            _same_thread: PhantomData,
        })
    }
    pub fn register_stats(&self) -> RegisterStats {
        let mut stats = RegisterStats::default();
        // SAFETY: a live creator-thread host and matching three-u64 C layout.
        unsafe { trames_hvf_register_stats(self.host.as_ptr(), &mut stats) };
        stats
    }
    pub fn features(&self) -> Features {
        features()
    }
    pub fn mapped_bytes(&self) -> usize {
        let granule = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
        self.spaces
            .values()
            .flat_map(|space| space.pages.values())
            .filter(|page| page.storage.strong_count() != 0)
            .map(|p| p.gpa & !(granule - 1))
            .collect::<BTreeSet<_>>()
            .len()
            * granule as usize
            + self
                .spaces
                .values()
                .flat_map(|s| s.blocks.values())
                .filter(|b| b.storage.strong_count() != 0)
                .count()
                * trames_memory::HARDWARE_MAPPING_CHUNK_SIZE as usize
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
        let identity = memory.instruction_identity().0;
        if !self.spaces.contains_key(&identity) {
            return Ok(());
        }
        self.activate(identity)?;
        for &base in self.spaces[&identity].blocks.keys() {
            check(unsafe { trames_hvf_block(self.host.as_ptr(), base, 0, 0) })?;
        }
        self.spaces.get_mut(&identity).unwrap().blocks.clear();
        let addresses: Vec<_> = self.spaces[&identity].pages.keys().copied().collect();
        for va in addresses {
            self.pte(va, 0, 0)?;
        }
        let space = self.spaces.get_mut(&identity).unwrap();
        space.pages.clear();
        space.chunks.clear();
        space.publications.clear();
        space.private_writable = 0;
        space.version = None;
        Ok(())
    }
    fn activate(&mut self, identity: u64) -> Result<(), Error> {
        if self.active != identity {
            #[cfg(feature = "diagnostics")]
            diagnostics::count(diagnostics::Count::RootSwitch, 1);
            check(unsafe {
                trames_hvf_space_select(self.host.as_ptr(), self.spaces[&identity].host.as_ptr())
            })?;
            self.active = identity;
        }
        self.clock = self.clock.wrapping_add(1);
        self.spaces.get_mut(&identity).unwrap().used = self.clock;
        Ok(())
    }
    fn select_view(&mut self, memory: &View) -> Result<(), Error> {
        let identity = memory.instruction_identity().0;
        if !self.spaces.contains_key(&identity) {
            let space = if let Some(space) = self.spaces.remove(&0) {
                space
            } else {
                let mut error = 0;
                let host = unsafe { trames_hvf_space_create(self.host.as_ptr(), &mut error) };
                check(error)?;
                AddressSpace {
                    host: NonNull::new(host).ok_or(Error::Host(-1))?,
                    blocks: BTreeMap::new(),
                    pages: BTreeMap::new(),
                    chunks: BTreeMap::new(),
                    version: None,
                    used: 0,
                    publications: BTreeMap::new(),
                    private_writable: 0,
                }
            };
            self.spaces.insert(identity, space);
        }
        self.activate(identity)?;
        // Evict only inactive roots. They retain weak physical aliases and must
        // be revalidated before selection; evicting metadata cannot pin RAM.
        while self.spaces.len() > CACHED_SPACES
            || self.spaces.values().map(|s| s.pages.len()).sum::<usize>() > CACHED_PAGES
        {
            let Some((&oldest, space)) = self
                .spaces
                .iter()
                .filter(|(id, _)| **id != identity)
                .min_by_key(|(_, s)| s.used)
            else {
                break;
            };
            check(unsafe { trames_hvf_space_destroy(self.host.as_ptr(), space.host.as_ptr()) })?;
            self.spaces.remove(&oldest);
        }
        Ok(())
    }
    fn synchronize_blocks(&mut self, memory: &View) -> Result<(), Error> {
        let space = self.spaces.get_mut(&self.active).unwrap();
        let version = memory.hardware_view_version();
        if space.version.map(|v| v.0) == Some(version.0) {
            return Ok(());
        }
        let mut retired = BTreeSet::new();
        if let Some(changes) = space
            .version
            .and_then(|v| memory.hardware_mapping_ranges(v.0))
        {
            for change in changes {
                let first = change.start & !(trames_memory::HARDWARE_MAPPING_CHUNK_SIZE - 1);
                retired.extend(space.blocks.range(first..change.end).map(|(&a, _)| a));
            }
        } else {
            retired.extend(space.blocks.iter().filter_map(|(&base, b)| {
                (b.storage.strong_count() == 0
                    || b.version != memory.hardware_mapping_chunk_version(base))
                .then_some(base)
            }));
        }
        for base in retired {
            check(unsafe { trames_hvf_block(self.host.as_ptr(), base, 0, 0) })?;
            space.blocks.remove(&base);
        }
        Ok(())
    }
    fn synchronize_view(&mut self, memory: &View) -> Result<(), Error> {
        #[cfg(feature = "diagnostics")]
        let measured = diagnostics::start(diagnostics::Work::Sync);
        self.select_view(memory)?;
        self.synchronize_blocks(memory)?;
        let version = memory.hardware_view_version();
        let space = self.spaces.get_mut(&self.active).unwrap();
        if space.version.map(|v| v.0) != Some(version.0) {
            let (addresses, mut chunks): (Vec<_>, Vec<_>) = if let Some(changes) = space
                .version
                .and_then(|v| memory.hardware_mapping_ranges(v.0))
            {
                #[cfg(feature = "diagnostics")]
                diagnostics::count(diagnostics::Count::Delta, 1);
                let mut ranges: Vec<_> = changes.collect();
                ranges.sort_unstable_by_key(|range| range.start);
                let mut addresses = Vec::new();
                let mut chunks = Vec::new();
                let mut end = 0;
                for range in ranges {
                    let start = range.start.max(end);
                    if start < range.end {
                        addresses.extend(space.pages.range(start..range.end).map(|(&va, _)| va));
                        let first = start / trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
                        let last = (range.end - 1) / trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
                        chunks.extend(space.chunks.range(first..=last).map(|(&key, _)| key));
                    }
                    end = end.max(range.end);
                }
                (addresses, chunks)
            } else {
                #[cfg(feature = "diagnostics")]
                {
                    diagnostics::count(diagnostics::Count::Fallback, 1);
                    diagnostics::count(diagnostics::Count::ChunkChecks, space.chunks.len());
                }
                // A CPU can miss arbitrarily many updates while parked or
                // running another mm. Compare persistent coarse stamps, then
                // inspect leaves only in changed chunks. No history-length
                // threshold may turn a tiny change into a full-mm page walk.
                let chunks: Vec<_> = space
                    .chunks
                    .iter()
                    .filter_map(|(&key, chunk)| {
                        let base = key * trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
                        (chunk.version != memory.hardware_mapping_chunk_version(base))
                            .then_some(key)
                    })
                    .collect();
                let mut addresses = Vec::new();
                for &key in &chunks {
                    let base = key * trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
                    addresses.extend(
                        space
                            .pages
                            .range(base..base + trames_memory::HARDWARE_MAPPING_CHUNK_SIZE)
                            .map(|(&va, _)| va),
                    );
                }
                (addresses, chunks)
            };
            #[cfg(feature = "diagnostics")]
            diagnostics::count(diagnostics::Count::Inspect, addresses.len());
            let mut retired = Vec::new();
            for va in addresses {
                let Some(page) = space.pages.get_mut(&va) else {
                    continue;
                };
                #[cfg(test)]
                {
                    self.translations.inspections += 1;
                }
                let info = memory.hardware_page_info(va).ok();
                let Some(info) = info.filter(|info| {
                    page.storage.strong_count() != 0 && info.storage_id == page.info.storage_id
                }) else {
                    retired.push(va);
                    continue;
                };
                let flags = Self::flags(info)?;
                if flags != Self::flags(page.info)? {
                    check(unsafe { trames_hvf_page(self.host.as_ptr(), va, page.gpa, flags) })?;
                    #[cfg(test)]
                    {
                        self.translations.updates += 1;
                    }
                }
                // Contents and code freshness do not change a translation.
                if info.permissions.execute
                    && (!page.info.permissions.execute
                        || info.instruction_revision != page.info.instruction_revision
                        || info.backing_version != page.info.backing_version)
                {
                    self.machine.publish_code(page.gpa)?;
                }
                let previous = page.write;
                page.write = if info.writable {
                    Some(memory.hardware_write_frame(va)?)
                } else {
                    None
                };
                page.info = info;
                let write = page.write;
                space.track_write(va, previous, write);
            }
            #[cfg(feature = "diagnostics")]
            diagnostics::count(diagnostics::Count::Retire, retired.len());
            for va in retired {
                check(unsafe { trames_hvf_page(self.host.as_ptr(), va, 0, 0) })?;
                let old = space.remove_page(va).unwrap();
                space.track_write(va, old.write, None);
                #[cfg(test)]
                {
                    self.translations.retired += 1;
                }
            }
            chunks.dedup();
            for key in chunks {
                if let Some(chunk) = space.chunks.get_mut(&key) {
                    chunk.version = memory.hardware_mapping_chunk_version(
                        key * trames_memory::HARDWARE_MAPPING_CHUNK_SIZE,
                    );
                }
            }
        }
        // Conservative hardware dirty publication is once per joined epoch,
        // rather than a page walk at every short syscall in that epoch.
        if space.version != Some(version) {
            if space.private_writable != 0 || !space.blocks.is_empty() {
                memory.mark_hardware_private_writes();
            }
            for &write in space.publications.values() {
                memory.mark_hardware_frame_write(write)?;
            }
        }
        space.version = Some(version);
        #[cfg(feature = "diagnostics")]
        diagnostics::end(diagnostics::Work::Sync, measured);
        Ok(())
    }
    #[inline(never)]
    fn try_native_range(&mut self, memory: &View, va: u64) -> Result<bool, Error> {
        let error = std::cell::Cell::new(None);
        // SAFETY: same hardware-only actor/retained canonical owner contract.
        let range = unsafe {
            memory.export_hardware_range(va, || {
                self.machine.allocate_range().map_err(|e| {
                    error.set(Some(e));
                    MemoryError::BackingIo(5)
                })
            })
        }
        .map_err(|e| error.take().unwrap_or(Error::Memory(e)))?;
        if let Some(storage) = range {
            let base = va & !(trames_memory::HARDWARE_MAPPING_CHUNK_SIZE - 1);
            // Promotion after a partial permission edit can leave sparse
            // leaf translations in this CPU's root. Withdraw those aliases
            // before replacing the leaf table with one uniform block.
            let leaves: Vec<_> = self.spaces[&self.active]
                .pages
                .range(base..base + trames_memory::HARDWARE_MAPPING_CHUNK_SIZE)
                .map(|(&va, _)| va)
                .collect();
            for address in leaves {
                self.pte(address, 0, 0)?;
                let space = self.spaces.get_mut(&self.active).unwrap();
                let old = space.remove_page(address).unwrap();
                space.track_write(address, old.write, None);
            }
            check(unsafe {
                trames_hvf_block(self.host.as_ptr(), base, storage.physical_address(), 3)
            })?;
            self.spaces.get_mut(&self.active).unwrap().blocks.insert(
                base,
                Block {
                    storage: Arc::downgrade(&storage),
                    version: memory.hardware_mapping_chunk_version(base),
                },
            );
            memory.mark_hardware_private_writes();
            #[cfg(test)]
            {
                self.translations.blocks += 1;
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn fault(&mut self, memory: &View, event: &HostExit) -> Result<(), Error> {
        #[cfg(feature = "diagnostics")]
        diagnostics::count(diagnostics::Count::Fault, 1);
        #[cfg(test)]
        {
            self.translations.faults += 1;
        }
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
        #[cfg(feature = "diagnostics")]
        diagnostics::count(
            match access {
                Access::Write => diagnostics::Count::FaultWrite,
                Access::Read => diagnostics::Count::FaultRead,
                Access::Execute => diagnostics::Count::FaultExecute,
            },
            1,
        );
        if !(4..=15).contains(&status) {
            return Err(Error::Fault {
                pc: event.pc,
                address: event.address,
                syndrome: event.syndrome,
            });
        }
        let va = (event.address & 0x00ff_ffff_ffff_ffff) & !(PAGE_SIZE - 1);
        if (VECTOR_VA..VECTOR_VA + 65536).contains(&va) {
            memory.check(event.address, 1, access)?;
            return Err(Error::Unsupported("reserved exception adapter address"));
        }
        let block_base = va & !(trames_memory::HARDWARE_MAPPING_CHUNK_SIZE - 1);
        if access != Access::Execute
            && block_base != VECTOR_VA
            && memory.hardware_ranges_enabled()
            && self.machine.native_ranges_enabled()
            && self.try_native_range(memory, va)?
        {
            return Ok(());
        }
        memory.check(event.address, 1, access)?;
        // SAFETY: the scheduler admits every View in hardware mode and joins
        // before coordinator mutation. Canonical exports gate all host borrows.
        let allocation_error = std::cell::Cell::new(None);
        let created = std::cell::Cell::new(false);
        let exported = unsafe {
            memory.export_hardware_page_with_mapping(
                va,
                access,
                |bytes| {
                    created.set(true);
                    #[cfg(feature = "diagnostics")]
                    diagnostics::count(
                        if bytes.is_none() {
                            diagnostics::Count::AllocateZero
                        } else {
                            diagnostics::Count::AllocateCopy
                        },
                        1,
                    );
                    self.machine.allocate(bytes).map_err(|error| {
                        allocation_error.set(Some(error));
                        MemoryError::BackingIo(5)
                    })
                },
                |mapping| {
                    created.set(true);
                    #[cfg(feature = "diagnostics")]
                    diagnostics::count(diagnostics::Count::ImportStorage, 1);
                    self.machine.import(mapping).map_err(|error| {
                        allocation_error.set(Some(error));
                        MemoryError::BackingIo(5)
                    })
                },
            )
        };
        let (storage, info) =
            exported.map_err(|error| allocation_error.take().unwrap_or(Error::Memory(error)))?;
        #[cfg(feature = "diagnostics")]
        diagnostics::count(
            if created.get() {
                diagnostics::Count::FaultNewStorage
            } else {
                diagnostics::Count::FaultRetainedStorage
            },
            1,
        );
        self.install_page(memory, va, &storage, info)?;
        if !created.get() {
            // A migrated task already has canonical RAM. Fill only retained
            // neighbors in a bounded 1MiB block; never fault in unused mappings.
            let start = va & !((1 << 20) - 1);
            for address in (start..start + (1 << 20)).step_by(PAGE_SIZE as usize) {
                if (VECTOR_VA..VECTOR_VA + 65536).contains(&address)
                    || self.spaces[&self.active].pages.contains_key(&address)
                    || self.spaces[&self.active].blocks.contains_key(
                        &(address & !(trames_memory::HARDWARE_MAPPING_CHUNK_SIZE - 1)),
                    )
                {
                    continue;
                }
                if let Some((storage, info)) = memory.retained_hardware_page(address) {
                    if info.permissions.read {
                        self.install_page(memory, address, &storage, info)?;
                        #[cfg(feature = "diagnostics")]
                        diagnostics::count(diagnostics::Count::NeighborInstall, 1);
                    }
                }
            }
        }
        Ok(())
    }
    fn install_page(
        &mut self,
        memory: &View,
        va: u64,
        storage: &Arc<dyn HardwareStorage>,
        info: HardwarePageInfo,
    ) -> Result<(), Error> {
        let base = va & !(trames_memory::HARDWARE_MAPPING_CHUNK_SIZE - 1);
        if self.spaces[&self.active].blocks.contains_key(&base) {
            check(unsafe { trames_hvf_block(self.host.as_ptr(), base, 0, 0) })?;
            self.spaces
                .get_mut(&self.active)
                .unwrap()
                .blocks
                .remove(&base);
        }
        let flags = Self::flags(info)?;
        if info.permissions.execute {
            self.machine.publish_code(storage.physical_address())?;
        }
        self.pte(va, storage.physical_address(), flags)?;
        #[cfg(test)]
        {
            self.translations.exports += 1;
        }
        let write = if info.writable {
            Some(memory.hardware_write_frame(va)?)
        } else {
            None
        };
        let space = self.spaces.get_mut(&self.active).unwrap();
        let old = space.pages.insert(
            va,
            Page {
                gpa: storage.physical_address(),
                storage: Arc::downgrade(storage),
                info,
                write,
            },
        );
        if old.is_none() {
            let key = va / trames_memory::HARDWARE_MAPPING_CHUNK_SIZE;
            space
                .chunks
                .entry(key)
                .or_insert_with(|| CachedChunk {
                    pages: 0,
                    version: memory.hardware_mapping_chunk_version(va),
                })
                .pages += 1;
        }
        // Installing one leaf cannot acknowledge changes to other old leaves
        // in the chunk. Only synchronize_view advances an existing stamp.
        space.track_write(va, old.and_then(|page| page.write), write);
        #[cfg(feature = "diagnostics")]
        {
            diagnostics::count(diagnostics::Count::Export, 1);
            diagnostics::count(diagnostics::Count::PeakLeaves, space.pages.len());
        }
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
                "raw execution requires an explicit SME context",
            ));
        }
        // Baseline callers have no extended register owner and cannot enable SME.
        check(unsafe { trames_hvf_sme_input(self.host.as_ptr(), std::ptr::null(), 0) })?;
        self.run_parallel_inner(state, memory, quantum)
    }
    /// The movable task owns registers; the creator-thread executor owns only
    /// its private EL1 mailbox. Ordinary tasks allocate no extension buffer.
    pub fn run_parallel_sme(
        &mut self,
        state: &mut Aarch64State,
        registers: &mut Option<Box<SmeState>>,
        vl: u16,
        memory: &mut View,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        if registers
            .as_ref()
            .is_some_and(|r| !r.valid() || r.vl != vl as u64)
        {
            return Err(Error::Unsupported("invalid SME context"));
        }
        // SAFETY: the stopped creator-thread CPU copies the bounded context;
        // it retains no pointer into task storage while executing guest code.
        check(unsafe {
            trames_hvf_sme_input(
                self.host.as_ptr(),
                registers.as_deref().map_or(std::ptr::null(), |r| r),
                vl as u64,
            )
        })?;
        let result = self.run_parallel_inner(state, memory, quantum);
        // Capture on every exit, including a guest fault that may deliver a
        // signal. Immediate handled faults never copy through movable storage.
        let active = unsafe { trames_hvf_sme_active(self.host.as_ptr()) };
        if active != 0 {
            #[cfg(feature = "diagnostics")]
            {
                diagnostics::count(diagnostics::Count::SmeContext, 1);
                diagnostics::count(
                    diagnostics::Count::SmeStreaming,
                    usize::from(active & 1 != 0),
                );
            }
            let saved = registers.get_or_insert_with(|| Box::new(SmeState::default()));
            unsafe { trames_hvf_sme_output(self.host.as_ptr(), saved.as_mut()) };
            if matches!(
                result,
                Ok(ExecutionExit {
                    reason: ExitReason::Syscall,
                    ..
                })
            ) {
                saved.syscall();
            }
            if saved.svcr == 0 {
                *registers = None;
            }
        } else {
            *registers = None;
        }
        result
    }
    pub fn run_sme(
        &mut self,
        state: &mut Aarch64State,
        registers: &mut Option<Box<SmeState>>,
        vl: u16,
        memory: &mut Memory,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        memory.prepare_hardware()?;
        let mut view = memory
            .parallel_view(&mut self.registry)
            .ok_or(Error::Unsupported("hardware view admission"))?;
        view.enable_hardware();
        let result = self.run_parallel_sme(state, registers, vl, &mut view, quantum);
        drop(view);
        memory.commit_parallel()?;
        result
    }
    fn run_parallel_inner(
        &mut self,
        state: &mut Aarch64State,
        memory: &mut View,
        quantum: Duration,
    ) -> Result<ExecutionExit, Error> {
        self.wait = None;
        self.synchronize_view(memory)?;
        let deadline = std::time::Instant::now() + quantum;
        let mut continue_fault = false;
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
            #[cfg(feature = "diagnostics")]
            let measured = diagnostics::start(diagnostics::Work::Run);
            check(unsafe {
                trames_hvf_run(
                    self.host.as_ptr(),
                    state,
                    left.as_nanos().min(100_000_000) as u64,
                    continue_fault,
                    &mut event,
                )
            })?;
            #[cfg(feature = "diagnostics")]
            diagnostics::end(diagnostics::Work::Run, measured);
            continue_fault = false;
            match event.kind {
                0 | 1 => {
                    return Ok(ExecutionExit {
                        reason: if event.kind == 1 {
                            ExitReason::Syscall
                        } else {
                            ExitReason::Yield
                        },
                        instructions: InstructionCount::Unavailable,
                    });
                }
                2 => {
                    #[cfg(feature = "diagnostics")]
                    let measured = diagnostics::start(diagnostics::Work::Fault);
                    self.fault(memory, &event)?;
                    // This handler edits memory/PTEs, never the saved EL0 bank.
                    // Reuse the EL1 context and the slice's absolute timer.
                    continue_fault = true;
                    #[cfg(feature = "diagnostics")]
                    diagnostics::end(diagnostics::Work::Fault, measured);
                }
                4 => {
                    #[cfg(feature = "diagnostics")]
                    let measured = diagnostics::start(diagnostics::Work::Resume);
                    let wait = self.machine.resume(event.address)?;
                    #[cfg(feature = "diagnostics")]
                    diagnostics::end(diagnostics::Work::Resume, measured);
                    if let Some(wait) = wait {
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
                    });
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
