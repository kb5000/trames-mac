# Hardware execution and pageable memory

`trames-mac` is a terminal product over the same `trames-session` Linux userspace
kernel as `trames-ipad`. ARM64 instructions execute in Hypervisor.framework; a
small EL1 adapter returns SVC, faults and timer exits to the host kernel. No full
Linux kernel, boot RAM image, interpreter, or product-specific syscall kernel is
started. [Issue 44](https://github.com/kb5000/trames-ipad/issues/44) tracks remaining
product integration gates.

## VM and CPU ownership

There is one HVF VM per host process. Each existing execution worker lazily
creates its own thread-affine vCPU. Stage-one roots persist per Linux address
space on that worker, across task switches and syscall/epoch boundaries. Movable Linux
task state includes all general/SIMD registers, NZCV, stack and TLS; it never
moves a vCPU between threads. A distinct ASID and non-global PTEs isolate each
worker's virtual translations. A root switch always invalidates that worker's
ASID before EL0 resumes; cached roots consume no additional ASIDs. Table updates
invalidate that ASID, rather than all peers' translations. ASID encoding and TLBI operands follow the ARM64 layout
also used by [Linux](https://github.com/torvalds/linux/blob/v6.12/arch/arm64/include/asm/mmu_context.h).

All CPUs share canonical physical storage exported from the common frozen
`View`. Threads, processes and MAP_SHARED providers therefore see the original
bytes, not per-vCPU RAM shadows. The common runtime owns permission/COW/quota
admission and dirty/persistence publication. Task switches clear exclusive
reservations; ordinary instructions use hardware atomicity and memory ordering.
Cache-maintenance aborts follow the [Linux arm64 CM/WnR distinction](https://github.com/torvalds/linux/blob/v6.12/arch/arm64/mm/fault.c):
clearing an RX instruction cache is not an ordinary guest write.
Hardware instruction retirement remains unavailable. Scheduler fairness uses
bounded slices without manufacturing instruction counts.

The VM lifetime factory acknowledges final teardown before allowing a new VM.
Host byte borrows retain the VM. vCPU destruction occurs on its creator thread;
idle PTE caches hold weak storage references and invalidate them before reuse.
Retained data can be rebound after the previous VM has permanently ended.

The cache retains at most eight roots and 131,072 cached leaf descriptors per
worker, evicting inactive least-recently-used roots first. A single active space
can exceed the metadata target; the limit does not invalidate its working set.
Cached leaves hold weak physical owners, so dead address spaces cannot pin guest
RAM. Their bounded page-table metadata is freed on eviction or vCPU destruction.

Mapping tokens and bounded address-range deltas come from the shared runtime.
Unchanged mappings do not trigger a page inspection. A one-page permission
change checks that page and updates its descriptor in place. Missing/replaced
storage retires the leaf; content-only changes preserve it. Expired history uses
persistent chunk stamps to narrow revalidation; a full snapshot replacement
changes all live stamps. Conservative dirty
publication runs once per joined write epoch. Canonical private data uses one
VM-dirty notification; only provider/code hooks need per-page indices. It remains
conservative, not hardware dirty-bit tracking. Fork still has the existing eager writable detachment limitation.

## Linux register access

EL0 can read `CNTVCT_EL0` and `CNTFRQ_EL0` directly, using the hardware counter
and its real frequency across vCPUs and scheduler slices. `CNTKCTL_EL1` enables
only the virtual counter, following [Linux's ARM64 timer setup](https://github.com/torvalds/linux/blob/v6.12/drivers/clocksource/arm_arch_timer.c#L921-L942); guest code cannot
control the scheduler's timer. Counter reads require no emulation or added
synchronization. Trapped privileged system-register accesses become recoverable
Linux `SIGILL` with the faulting PC, allowing the guest's own signal handler to
continue. They no longer terminate the execution backend.

The static-musl `cpu-access.c` fixture checks four concurrent readers, elapsed
time against `CLOCK_MONOTONIC` across sleep, and three privileged instructions
with edited signal contexts. The identical ELF passes on native ARM64 Linux and
on HVF with one or four execution slots. See the
[compatibility and CoreMark checkpoint](../validation/mac-compat-coremark.md).

## Host borrows

Ordinary guest transfers use audited raw instructions through the common memory
API, without creating guest Rust references or toggling stage-two permissions.
Metadata-only export retains the existing physical handle without a byte gate.
Actual host code writes publish the changed range; guest IC/DC maintenance,
initial executable mapping and page-in keep their existing cache responsibilities.
A process-global code epoch no longer makes each CPU flush its executable image.

Before a syscall/native/VFS callback can expose a Rust byte slice, its physical
host granule loses stage-two hardware access. The original 64-byte region locks
still arbitrate overlapping host borrows; unrelated granules and CPUs continue.
The last borrower restores access and wakes affected tasks. Hardware stage-two
faults park on the existing epoch queue and return their execution slots.
Release-before-park and stop/checkpoint races use one-shot readiness tickets.
There is no additional resource-service thread or polling message bus.

Hardware views bypass old native MMU aliases. The joined preparation boundary
invalidates cached native translations before releasing those aliases. Storage
transfer happens once under all region bits. File-backed mappings with explicit retained allocation geometry can be imported
at their original addresses. All 4KiB pages within a host granule share one
physical mapping and borrow gate; the descriptor preserves display aliases when
a VM is replaced. Opaque raw GPU/pinned mappings are still rejected until they
have a compatible ownership protocol; shared providers without canonical
`shared_page()` exports are rejected as well. Ordinary shared file and anonymous
mappings are supported. See the common runtime's
[region-memory contract](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/region-memory.md).

## Backend capabilities and retained translations

The generic CPU interface supplies runtime feature masks with conservative
constant defaults, preserving interpreter/iPad behavior. HVF reads the vCPU
configuration's ID registers and advertises only supported extensions whose
state fits the existing GPR/128-bit SIMD/FPCR/FPSR migration and signal model:
FP/ASIMD, AES/PMULL/SHA/CRC, LSE, FP16, RDM, dot-product, FHM, I8MM and BF16.
Unsupported entries stay absent. CPUID, SVE, SME, PAC and extra PSTATE state are
not advertised. The instruction fixture executes every advertised subset and
checks results; physical-host capabilities alone are not an admission oracle.

Page-table ownership uses an address-space-local GPA hash index rather than a
linear list walk. Sparse removal and reused GPAs update the index and cleanup
list together. No shared guest execution lock is added. A migrated task can fill
missing PTEs from already resident, already charged hardware owners in the same
bounded 1 MiB virtual block. This never loads, allocates or charges untouched
neighbors, and retains the same permission, code-publication and ownership hooks.

Hardware mapping history stores coalesced address intervals. A contiguous change
larger than 4096 pages no longer discards incremental history. Each CPU intersects
those intervals with its own cached leaves, merging overlaps; it never enumerates
untouched pages or builds another full ordered page set. When bounded history
expires, a CPU compares persistent 2 MiB chunk stamps and inspects only cached
leaves in changed chunks. This avoids a whole-mm walk merely because a worker
was parked, migrated or delayed by peers. New leaves cannot acknowledge changes
to unchecked old leaves. Empty chunks retain no stamp/physical owner; full
snapshot replacement invalidates every live chunk. Signed acceptance checks
both precise interval invalidation and an 80-update gap with zero unrelated
leaf checks, followed by exact protection, unmap and address-reuse behavior.
The shared runtime builds no chunk stamps for the iPad software interpreter.

## Physical paging

The default host-managed policy grows from the ordinary host VM pool. Numeric
`--memory-mib N` remains an optional logical resident-byte cap. Neither policy
preallocates a RAM image; only accessed pages acquire physical slots. Four unrelated 4KiB guest pages
can share one 16KiB host granule, and partially freed granules are reused before
allocating another. Mapping addresses need not be contiguous.

Each host execution thread keeps one weak partial-granule hint. Atomic slot
claims arbitrate it against the weak shared pool; an acquire claim pairs with
the previous slot's release on ownership handoff. Only the shared pool metadata
uses its short mutex. HVF mapping and disjoint 4 KiB initialization run after
that lock is released, so unrelated CPUs do not queue behind either operation.
No idle cache pins RAM or a dead VM. Hints are checked against the current VM
identity, and last-owner unmap/reclamation remains immediate. The signed
12-thread test verifies unique live physical addresses, complete page contents,
reuse among retained peers and zero mapped/backing bytes while TLS hints live.

Private canonical storage uses anonymous macOS VM arenas. One arena reserves
1 MiB of host virtual and linear IPA addresses. HVF registers that address range
in one call; registration does not populate untouched host pages on the tested
macOS build. Physical byte ownership and initialization remain per used 16 KiB
host granule, with no guest page descriptors, data charge or readable guest PTEs
for untouched arena slots. The pool retains weak arena handles and reuses partial
granules first. Explicit shared-file imports retain their original owner and
alias geometry; page tables/imports use a separate granule-size address pool.

An arena bitmap distinguishes HVF address registration from byte ownership.
Per-granule pressure/final release still withdraws its stage-two mapping, then
preserves live data through the native pager or marks dead storage reusable.
Reaching an unmapped hole registers only that granule; it cannot disturb a live
peer. Last-owner teardown coalesces remaining untouched registrations before
releasing the host range and IPA reservation. Failed withdrawal preserves the
allocation/address reservation rather than allowing a dangling HVF alias.
Whole-arena registration runs outside the allocator mutex, and no idle strong
pool owner retains RAM or a dead VM.

A bounded direct HVF probe registered 128 MiB with zero data pages resident,
then observed one 16 KiB granule after a host/guest access. A partial unmap kept
neighbor mappings usable. Signed acceptance measures one resident granule after
initial 1 MiB registration, checks partial free/reuse and pressure holes/page-in,
complete data, linear addresses across table growth and zero live-byte counters
after final release. This is lazy address registration, not eager VMA population
or a cache of free wired pages. Paging behavior is measured on the installed OS;
Apple's mapping API specifies address ranges, not physical-residency guarantees.

Reclamation withdraws the stage-two mapping and uses Darwin MADV_DONTNEED to
deactivate residency while retaining live contents at the same host address.
Darwin maps this hint to VM_SYNC_DEACTIVATE, without VM_SYNC_KILLPAGES; it must
not be ported to Linux's destructive anonymous DONTNEED semantics. The native
macOS pager can compress/swap these pages, and later host/HVF access restores
them. Trames no longer preallocates an application page file or performs
synchronous MS_SYNC writeback when reclaiming ordinary guest RAM. A successful
hint permits reclaim; it does not guarantee an immediate RSS reduction.
See [XNU's implementation](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/vm/vm_map.c).

`backing_bytes` now counts live private host-granule storage, including padding;
it is neither disk usage nor exact host residency. `mapped_bytes` counts used
host granules currently bound to HVF, excluding untouched address registrations;
`page_out_bytes` counts withdrawn used bindings and `page_ins` their rebindings.
Logical guest-page accounting remains in the common runtime. Virtual VMA length
and protection changes do not consume an explicit guest RAM budget; actual file
access and physical hardware exports do. Dense accessible-page metadata has a
separate 16 GiB ceiling and still consumes host memory. Fork accounting remains
conservative per VM rather than unique-physical-page/PSS accounting.

The supported Dispatch memory-pressure source latches only level metadata on a
shared system utility queue. The existing joined session-control hook performs
eviction in bounded batches: warning targets one quarter of mapped RAM for
reclamation, critical targets three quarters. Normal-pressure events cancel
pending work. No application monitor thread is created. The obsolete EVFILT_VM
filter is deliberately not used: current macOS returns ENOTSUP for it.

`--resident-mib N` sets an optional deterministic hardware working-set target
(minimum 8MiB), independent of the logical guest budget. Mapping/page-in faults
trim cold granules with a small hysteresis. It is a soft working-set target,
**not a total process RSS cap**: kernel/scheduler metadata, host aliases, files,
and temporary COW preparation also consume memory. Frozen-view fork preparation
can still copy writable private pages before actual guest writes; reducing that
fork cost remains follow-up work.

## Local acceptance

Build the fixtures unchanged with a native AArch64 Linux static-musl toolchain:

```sh
musl-gcc -O2 -static -pthread tests/fixtures/smp-paging.c -o smp-paging
musl-gcc -O2 -static tests/fixtures/shared-map.c -o shared-map
musl-gcc -O2 -static tests/fixtures/code-cache.c -o code-cache
```

After `bash scripts/build-mac.sh`, run:

```sh
python3 tests/integration/mac-smp-paging.py \
  --runner target/release/trames-mac-run --fixture /path/to/smp-paging \
  --rootfs /path/to/verified-Alpine.tar.gz --output target/smp-paging-acceptance
```

The test compares four CPUs, a 16MiB working set, and one CPU. It checks four
million shared atomic increments, two complete 64MiB byte checksums, fork/COW,
and live-process RSS before/after reclamation. Shared-map and code-cache fixtures
add cross-process MAP_SHARED/VFS coherence and EL0 cache-maintenance/W^X checks.
Existing session/signals tests cover external commands, PTY/job control,
persistence, signal frames and precise fault recovery.

Run `python3 scripts/check-hardware.py` to build and sign both explicit test
binaries, then execute the hardware/pressure checks.

Ignored tests in the signed HVF test binary cover host-borrow parking, live dirty
page-out/page-in, retained storage across VM replacement, partial-granule reuse,
concurrent VM teardown, and retained file pixels across eviction and VM replacement.
The signed Mac product test binary additionally
injects process-local pressure metadata to verify latch-only polling followed by
joined eviction. It does not put other Mac applications under artificial memory
pressure. Hosted build CI does not claim nested Hypervisor access.

## Native Wayland presentation

The main thread owns a blocking CF/AppKit event loop. The existing Linux session
runs on one thread with its existing execution workers; vCPUs stay on their
creator threads. A coalesced compositor snapshot subscription signals the main
run-loop source after releasing the scene lock. There is no UI polling thread.
AppKit and Metal initialize only when the first mapped toplevel appears. Each
toplevel owns a native window. Its external CSD shadow is clipped to the native
content geometry; outlying subsurfaces and popups use bounded child rectangles instead of a desktop-sized transparent input surface. The last
window releases cached Metal setup and removes the Dock presence without
stopping Linux or its terminal.

Metal reads retained file-backed SHM mappings directly. No Rust byte slice or
copy of the guest framebuffer is introduced by the presenter. A snapshot lease
owns the source until GPU reads finish. Positive frame acknowledgement also
waits for every drawable's actual presentation; skipped, cancelled, and occluded
frames receive no successful presentation acknowledgement. Invalidated run-loop
sources remain retained until pending GPU callbacks finish, so teardown cannot
call a freed UI context. The Metal-backed view opts into AppKit layer updates.
Native geometry and drawable presentation share a Core Animation transaction;
the main thread waits for submission scheduling, not GPU completion. AppKit's
native run/stop lifecycle handles events after the initial headless CF loop. Frame cancellation and multi-drawable completion have
unit coverage independent of Hypervisor access.

Native input uses the common Wayland seat. Interactive move/resize is admitted
only with a live press serial belonging to the same toplevel. Native close sends
`xdg_toplevel.close`; the client controls its shutdown and other Linux processes
continue. Cmd-W closes the focused window; Cmd-Q ends the Linux session.
Host clipboard/IME integration and hardware external GPU exports remain separate
product gates. See the [window acceptance record](../validation/mac-windows.md).
Physical iPad release acceptance remains a separate product check.

The [recorded acceptance summary](../validation/hvf-smp-paging-summary.json)
contains exact source/binary/fixture hashes, CI scope and live RSS measurements.

## Whole-system backend selection review (2026-10-27)

Owner constraints: a full Linux-kernel VM is excluded. Thin HVF CPU/MMU execution
is allowed. The shared userspace Linux kernel and native GPU/NPU API integration
are defining capabilities. Mac changes must preserve iPad compatibility or stay
behind isolated execution/product backends. The memory/device ownership design
below remains a target; the first Mac NativeCall transport checkpoint is recorded
in [local validation](../validation/mac-windows.md#native-device-transport-checkpoint-2026-10-27).

Keep one common Linux/resource implementation. Use the existing iPad interpreter
and a thin Mac HVF executor as CPU frontends. Native instruction rewriting remains
an alternate Mac frontend experiment; selecting it must not require a second
Linux kernel or an unconditional resource-service IPC layer. HVF does create a
hardware virtualization context, but no Linux kernel or virtual device OS is
booted. Native device APIs execute in the host process, not in a virtual GPU.

| Layer | Shared responsibility | Product/backend isolation |
| --- | --- | --- |
| Linux runtime | Tasks, signals, namespaces, FD/OFD, VFS, sockets, futex, quotas | Host adapters where OS mechanisms differ |
| Memory/resources | Byte identity, permissions, COW, allocation lifetime, device leases, completion | HVF GPA/page tables, interpreter translations, native API allocation requirements |
| Native calls | Negotiated guest ABI, authorization, opaque handles, command semantics, waits | Interpreter event, HVF exit, or future native trampoline transport |
| Presentation | Wayland scene/input protocol and retained image capabilities | UIKit on iPad; AppKit/Metal presentation on Mac |

The existing [CoreMark comparison](../validation/mac-compat-coremark.md) establishes
near-native compute for its workload. [Firefox CPU sampling](../validation/mac-windows.md#cpu-baseline)
finds most CPU in permission changes and mapping maintenance. Neither isolates
the intrinsic cost of an unchanged-mapping HVF syscall/device-call exit.

### Common memory and native device contract

The target is no hardware permission toggle for metadata-only retention or
ordinary bounded syscall transfers, and no full cached-PTE walk on an unchanged
address space. Stable physical mappings should last while resident; translation
state should belong to the Linux address space, with actual mapping, permission,
COW and reclamation changes applied incrementally. Cache retention must be
bounded, including inactive address spaces and ASID reuse.

Code-content freshness is distinct from translation identity. The hardware
backend now preserves translations across content changes and uses producer-side
code publication rather than a global code epoch to refresh all executable pages.
Fork should eventually detach private storage on first write rather than eagerly
at View admission. None of these changes may introduce per-load bookkeeping or
new ordinary-memory barriers.

Guest bytes and host Rust objects need distinct access contracts. Metadata-only
handle retention does not imply byte borrowing. Audited raw copy-in/out now
handles ordinary concurrent guest transfers; existing APIs
that create guest Rust slices must continue excluding conflicting hardware
access until their contract is changed and validated. Physical lifetime, truncation and architectural ordering
remain necessary, using the same guest permission/concurrency model on both
products.

For device memory, prefer compatible host allocation first, then map that storage
into the guest. A common descriptor should retain allocation identity, full
geometry/alignment, offset/range, guest mappings, native owner, accounting and
in-flight completion leases. Guest ABI handles are capabilities, never raw host
object/function pointers. Small control records become owned host values; large
payloads use retained storage where the API permits it. Arbitrary scattered
guest pages are not automatically one native buffer: Metal's
[bytesNoCopy contract](https://developer.apple.com/documentation/metal/mtldevice/makebuffer(bytesnocopy:length:options:deallocator:))
requires page-aligned geometry inside a single host VM region.

Submissions retain only affected resources until actual completion. Logical
unmap must not recycle storage still used by a device. Reclaim idle/unleased
storage normally; account live device memory separately. Use original API
barriers, queue order, fences and object synchronization, not a whole-guest pause
or a global native-call lock. Completion wakes the existing task queue. Small
calls run on the calling worker; blocking/async work releases its execution slot.
Do not introduce a polling bus or new coordinator/service thread.

Mac now offers opt-in `--vulkan` transport into the existing common Vulkan
ICD/dispatcher. The CPU recognizes the negotiated HLT/profile after a hardware
exception; the common registry still authorizes the complete stub, capability
snapshot and operation. Unknown opcodes/profiles retain guest traps. No second
GPU dispatcher or service thread is introduced. Compatible direct buffer import
remains follow-up work. Pure libc acceleration is separate: original ARM64 libc
already runs natively under HVF.
The existing Vulkan HOST_COHERENT path still uses shadow/baseline synchronization;
general direct CPU aliasing of native device allocations is not implemented.
Native texture presentation already has a separate no-readback lease path.
Changing the CPU frontend alone does not remove coherent-buffer transfers.

Batch command recording where API semantics allow, flushing before required
observable results and synchronization. Measure crossings per submission and
parameter conversion, not only execution time on the GPU. NPU/Core ML integration
is currently planned, not a production device backend. Prefer a model/execution
provider boundary; Core ML's
[compute-unit selection](https://developer.apple.com/documentation/coreml/mlcomputeunits)
does not promise that every model/operator executes on ANE or avoids conversion.

### Windowed, headless and container costs

Native compute and presentation are independent capabilities. A headless GPU/NPU
container must not require a Wayland server, CAMetalLayer, window, or desktop.
CPU-only servers should create neither device resources nor presenter resources.
The current automatic Mac mode defers AppKit/Metal presenter setup until the
first mapped window. `--no-windows` now skips Wayland and directly runs the same
session kernel on the main thread, without creating the presenter CF run-loop
source or a separate Linux session thread. Frameworks remain linked; this is not
a claim that all GUI framework pages are absent from the process. Native compute
initializes its own driver on demand, independently of presentation.

GUI cost should scale with visible changed surfaces: retained SHM/native textures,
coalesced notifications, real presentation completion and actual input. The
current presenter already consumes retained SHM/native textures without mandatory
pixel readback. Cache imports/pipelines where lifetime permits and avoid
recreating unchanged native wrappers or submitting unchanged frames. A display
lease ends only after required GPU/presentation completion; hiding a window must
not block unrelated server tasks.

Containers are namespace/resource objects in the same runtime, not a CPU pool,
presenter, full RAM allocation or native-service process per container. Reuse
immutable image/file data and shared resources; separate writable state and
accounting. Idle-container overhead should be bounded metadata; verify that
wakeups and mapping work track active/changed objects rather than scanning every
container at each short call. GPU model/resource caches need explicit owners,
budgets and release rules rather than one uncontrolled cache per container.

### Disk, network and frontend selection

Keep one byte authority for file I/O and shared mmap, including truncate/EOF,
open-unlinked lifetimes, quotas and durability. CPU backend replacement does not
remove gather/scatter or file-cache coherence. Small I/O can use reusable bounded
buffers; large I/O needs a retained-buffer/pinning/cancellation contract. Eligible
bulk reads already offload through the existing scheduler. Large writes, growth,
append, sync and namespace commits retain owner/service paths; releasing execution
slots for their potentially blocking work is a target, not completed coverage.
FileProvider retains streaming export, staged edits and conditional writeback;
external writers must not bypass live namespace/mapping ownership.

Ordinary root TCP/UDP already use nonblocking host BSD sockets, with no extra
packet-stack/NAT hop. Preserve shared OFD/socket objects and readiness parking;
explicit isolated container networks retain their own packet/network semantics.
Native per-guest Mac processes could inherit some host FDs, but current Arc/Mutex
objects, namespace ownership and device capabilities cannot be shared unchanged.
They require process-safe state or IPC. Same-process native execution instead
needs a way to represent overlapping guest virtual addresses. A syscall
trampoline alone solves neither issue.

Select an alternate frontend only with whole-workload evidence: headless idle
and loaded containers, syscall/AF_UNIX/futex wake tail latency, shared FD/mmap,
fork/COW, disk durability/bulk transfer, native compute submission/completion,
JIT, visible-window input/presentation, and post-completion reclamation. Record
CPU and blocked time separately, plus native crossings, copied bytes, retained
device bytes, RSS/footprint and recovery latency. The anonymous native-pager
path removes the former application page-file high-water mark; OS compression
and swap remain host-wide resources, not guest quota measurements.

Both products must pass the affected common semantic tests. Common ABI/memory
changes also require an iPad target check and relevant interpreter/native peers;
HVF-specific mapping tests and AppKit tests stay on Mac. Reuse unchanged passing
scopes. This documentation change adds no execution backend, device capability
or unsafe memory implementation; raw experiments remain in ignored local output.


### One registration domain across dispatch modes

The common session supplies its `parallel::Registry` to the serial hardware
CPU hook as well as worker View preparation. The Mac CPU uses that registry
when admitting a solo execution View, then commits normally at the joined
boundary. Its standalone `Executor::run` keeps an independent registry for
non-session callers. Runnable-count transitions therefore preserve mapping
metadata and code/alias ownership instead of rebuilding every page. The default
common hook calls the interpreter's original direct Memory execution, so this
backend-specific admission does not alter the iPad CPU path.


### EL1 exit transport namespace

The exception vector uses private `HVC #0x5452` to report SVC, abort and timer
state to the host. `HVC #0` is not a transparent register-preserving transport:
Apple's VM service namespace includes x0 values beginning at `0xc1000000`.
The virtualization stack can consume such a call, replace x0 and resume EL1
without returning an exception to this executor. Ordinary Linux pointers and
syscall arguments can have those same values.

The adapter validates the exit immediate before interpreting the saved EL0
exception. Pending TLBI now runs in the ordinary restore entry without another
HVC/host round trip. No argument register
is repurposed to carry an exit tag, and the change adds no extra guest exception
or ordinary-memory synchronization. Signed hardware tests preserve all general
registers across SVC for platform service-number values and high-bit aliases.
The original-musl fixture maps, writes and unmaps pages on both sides of the
service-number boundary and above 4 GiB.

Apple's [VMAPPLE header](https://github.com/apple/darwin-xnu/blob/main/pexpert/pexpert/arm64/VMAPPLE.h)
defines the namespace. A [QEMU developer's first-hand report](https://lists.gnu.org/archive/html/qemu-arm/2025-10/msg00553.html)
also describes the x0=`0xc1000000`, HVC-0 call being handled inside the
virtualization stack. The local boundary regression demonstrates this behavior
on the tested Mac rather than assuming that every HVC reaches the VMM.

The Mac product configures 1,024 guest task slots through the shared session
kernel; the common/iPad default remains 256. These count Linux threads and
processes, not host CPU workers. Jobs, stacks and memory remain demand allocated,
the CPU worker setting is unchanged, and the configured guest RAM budget still
applies. Clone admission, RLIMIT_NPROC and proc task-limit reports use the same
session capacity.

## Global optimization review (2026-10-28)

This is a code-based ordering of architectural opportunities, not a measured
ranking of current Firefox CPU time or a promised speedup. The selected design
remains a thin HVF executor over the shared userspace Linux kernel. Common
memory, scheduler and VFS changes must also support the iPad interpreter; HVF
register residency and AppKit presentation stay behind backend boundaries.

A passive check of the post-crash-fix browser, with no automated input or
navigation, used 11.78 ms of host-accounted process CPU in 10.01 s without a
profiler. A separate 20.70 s stack sample mostly observed blocked threads.
Neither interval measures active browsing, page-load latency, guest instruction
throughput or displayed-frame latency. Earlier active profiles predate the
platform-hypercall repair and must not be presented as post-fix CPU shares.
Private raw samples remain outside the repository.

### 1. Separate local address-space edits from backing-layout changes

The current run queue starts a mapping checkpoint with the caller's address
space, then computes a transitive closure through every exported backing key.
`BackingIndex::commit_domains` includes all versioned backing domains, including
read-only mapped files. `Domains::closure` can therefore connect processes
through a shared library even when the pending operation only changes a local
anonymous mapping. `vm_checkpoint` joins and prepares every participating space.
This is a verified amplification mechanism; its frequency in active browsing
has not yet been measured.

Classify effects before choosing the pause set: local mapping/permission edits
need their own address space; actual provider layout, EOF or storage replacement
needs affected aliases; ordinary shared-byte writes use canonical storage and
hardware coherence. Retained storage ownership must make those distinctions
safe. Simply deleting backing keys would break the existing lifetime and
provider-rebinding boundary.

Validate with independent processes sharing a read-only library: a local
anonymous mmap/mprotect/munmap must leave peers executing, while truncate and
storage replacement retain correct live-alias behavior. Record requested versus
quiesced address spaces, peer pause duration and progress outside the pause set.

### 2. Make backing refresh proportional to changed ranges

A changed mapping-domain generation still adds all that domain's mapped pages
to the snapshot candidate set. Each candidate gets a version/EOF check. Stable
direct data frames can refresh metadata without replacement, but the scan
remains. `FileMapping::changed` advances both page and file-domain generations;
content, layout and persistence invalidation still overlap.

Separate allocation/layout/EOF changes, code/monitor invalidation and durability
dirtiness. A bounded per-backing changed-range journal can reuse the existing
incremental snapshot and hardware mapping-delta mechanisms, with a full fallback
only when history expires or a provider cannot describe changes. Ordinary
canonical data needs no software publication to become visible. Preserve
executable aliases, interpreter cache rules, truncation and actual writeback.

Acceptance should hold mapped-file size constant while varying changed pages,
and then hold changed pages constant while increasing mapped-file size. Reuse
`backing_domain_checks`, `backing_page_checks`, `incremental_pages` and mapping
reconciliation counters to verify that unchanged pages do not dominate work.

### 3. Keep the active task's registers resident on its vCPU

`trames_hvf_run` currently sets and gets all 31 general and 32 SIMD registers on
every run: 126 framework register calls before counting system/FP state. A short
worker-local syscall still pays this full round trip. Persistent worker vCPUs
already provide the necessary ownership location.

For same-task continuation, materialize syscall inputs and changed results,
while leaving unaffected registers resident. Full state must still be available
for task switches/migration, signals and ucontext edits, fork, debugging and
joined ownership transfers. TLB maintenance and cancellation must respect that
resident-state contract. Keep the common CPU interface's fully materialized
fallback for the interpreter and other backends.

Measure framework register calls per syscall, same-task continuation rate and
active syscall-heavy throughput. Include all-register signal/migration tests;
the existing platform-number-shaped x0 regression must continue to pass.

### 4. Defer private fork copies and distinguish physical accounting

Fork currently charges each child's resident pages to its new address-space
account. Shared writable private storage can detach during parallel preparation,
before any actual child write. `can_fork` conservatively requires another full
parent resident charge. The 4 GiB setting remains a logical budget, not eagerly
allocated RAM or a measurement of unique physical storage.

A retained physical-allocation owner can share private pages until a real write;
HVF can use read-only COW mappings, while the interpreter keeps compatible
software write admission. This is a semantic first-write fault, distinct from
reintroducing artificial dirty faults each execution epoch. Keep per-space
logical limits separate from any aggregate unique-allocation policy; do not
silently redefine the existing budget. Release a physical charge only when its
last mapping/device owner ends.

Measure actual copy bytes and unique retained allocations for fork-without-write
and sparse-child-write workloads. Check guest COW isolation, quota failures,
reclamation and retained native-device leases, rather than treating aliased RSS
as a unique physical-page count.

### 5. Unify file storage and increment persistence work

Native shared files already provide direct mappings. Ordinary persistent files
can still create heap-backed mapped pages, then copy dirty pages back to their
underlying file. The non-overlay filesystem checkpoint also serializes the
whole namespace. Existing exclusions for unlinked and temporary files should
be retained.

Prefer a common retained allocation/cache owner for mmap and file I/O where the
storage backend supports it. Track dirty ranges and namespace deltas, with
periodic manifest snapshots. Large blocking transfers should release execution
slots through the existing service mechanism. Preserve append ordering, partial
I/O, error reporting and explicit fsync durability. Ordinary network and Unix
byte-stream operations already run on workers; remaining gather/scatter copies
are a separate ownership optimization, not a reason to add a resource thread.

### 6. Separate presentation costs from native rendering capability

The presenter already reads retained SHM/native textures without mandatory pixel
readback and uses coalesced notifications. It still creates no-copy Metal buffer
wrappers per frame and waits for command scheduling. Cache imports by allocation
and layout lifetime; investigate submission changes only while preserving actual
presentation acknowledgements, completion leases and resize/occlusion behavior.
Damage updates need retained content: arbitrary drawables cannot be assumed to
preserve earlier pixels.

The observed Firefox path remains software rendering. Metal presentation alone
does not establish native GPU WebRender support, and the current native Vulkan
compute path does not establish Firefox EGL/GL compatibility. Separately extend
the browser graphics API path and capability coverage. Native device allocations
that can also back guest mappings would remove the current HOST_COHERENT Vulkan
baseline scans; require compatible alignment, lifetime and guest-access contracts.
Keep native compute available without starting windows.

### Follow-up order and measurement boundary

Start with pause scope and backing-change classification together, since they
remove repeated cross-process work in both GUI and server workloads. Register
residency is the next isolated Mac executor opportunity; real COW and physical
accounting are a larger common-memory change. File persistence and native browser
graphics remain distinct projects with their own durability/capability gates.

For headless scaling, also audit the existing owner idle path's task/FD scans
and short sleeps. Extend existing wake subscriptions and deadlines instead of
introducing polling or a new coordinator thread. Attribute passive wakeups before
calling them a bottleneck.

Use a fixed offline browser workload to separate process CPU, pause time,
input-to-JavaScript and successful presentation latency, then verify real sites.
Report copied/scanned bytes and work amplification as well as elapsed time. No
implementation or performance improvement is claimed by this review. Runtime
source is unchanged, so existing acceptance results remain applicable.

## Implemented mapping and range checkpoint (2026-10-28)

The first two opportunities in the review above are now implemented in the
shared runtime: stable file providers no longer connect independent local VM
commits, and bounded changed-range history selects affected file-offset aliases.
Unknown providers and nonlocal layout/EOF changes retain conservative checks.
The [mapping-scope checkpoint](../validation/mac-windows.md#local-mapping-scope-and-backing-ranges-2026-10-28)
records the fixed-work Linux comparison, concurrency/EOF/COW regressions and
Mac/iPad acceptance. This does not implement register residency, lazy fork COW,
file persistence redesign or native Firefox GPU acceleration.

## Batched EL1 register transport (2026-10-28)

The executor now exchanges the complete 816-byte AArch64 state through a private
per-vCPU context. An EL1 entry restores GPRs, SIMD, SP, TLS, FP control/status and
NZCV; the exception adapter saves them before the existing private HVC. The host
copies the retained context directly. A normal SVC no longer makes 31 GPR and
32 SIMD framework get/set calls in both directions (126 calls). PC/CPSR and timer
control remain framework operations. The normal SVC still uses one hypercall.
There is no common CPU interface, Linux syscall implementation or iPad change.

Each initialized vCPU owns one anonymous host granule. Only its first 4 KiB is
mapped into EL1, inside the original 64 KiB adapter reservation. The mapping is
not executable and EL0 cannot read, write or execute it; no host pointer enters
guest memory. Every cached root of that vCPU maps the same context, while other
vCPUs retain separate physical owners. It is released after vCPU/root teardown;
a failed withdrawal retains the allocation rather than free/reuse a live alias.

The context has one execution owner. Host copies and opaque HVF calls alternate
on the creator thread, with no live host borrow during vCPU execution. Ordinary
ARM coherence and that execution boundary supply visibility; no additional byte
barriers, coordinator thread, polling bus or guest RAM publication is introduced.
The linked adapter image has fixed offsets and is copied intact, preserving its
internal branch offsets. C layout assertions match the shared architectural ABI.

Timer/cancellation exits in EL0 and stage-two faults retain the full framework
export fallback. An interruption before ERET keeps the complete input state,
including when only part of the actual bank was loaded. An interruption during
exception save completes only the bounded EL1 adapter with its timer masked; a
completion marker covers the final-save/HVC interval. It never runs arbitrary
EL0 code with an expired timer masked. Adapter faults are host failures. Signals,
fork, migration and native calls continue to receive fully materialized state.

The [register-context checkpoint](../validation/mac-windows.md#batched-register-context-2026-10-28)
records hardware, real Linux and fixed-work evidence. Additional resident-reload
prototypes did not show reliable benefit beyond batching in this workload and
were not included. The complete-state contract is retained.


## Opt-in HVF diagnostics

Default builds compile out the instrumentation. For attribution, build with
`--features trames-hvf/diagnostics`, ad-hoc sign the resulting executable with
`apps/macos/Trames.entitlements`, and set `TRAMES_HVF_PROFILE=1`. Each executing
host thread emits cumulative JSON to stderr at most once per active second and
on thread teardown. There is no shared counter lock, monitor thread or guest
load/store instrumentation. Profiling changes timings; use a normal signed
build without samplers for performance scores.

`wall_ns`, `cpu_ns` and `calls` follow this order: translation synchronization,
HVF run/adapter, stage-one fault, stage-two resume, allocation-pool lock wait,
HVF data map, slot initialization, final data unmap, arena recycling. Thread CPU
accounting includes host kernel time, not guest instruction retirement, and is
measured only for synchronization, run, fault, unmap and recycling. Other CPU
entries are unmeasured zeroes. Fault timings include nested allocation/map work;
do not add them as independent totals.

`counts` records journal fallback, interval synchronization, inspected cached
leaves, installed leaves, stage-one faults, retired leaves, root switches, peak
cached leaves per thread, fallback chunk-stamp checks, faults exporting new
storage, faults reusing retained storage, retained-neighbor PTE installations,
write/read/execute faults, zero/copy allocations and imports. Compare cumulative
deltas over workload boundaries; sum only final per-thread records for totals.
Historical diagnostic builds emitted only the first nine counters.


## Host-managed memory and fused translation maintenance

The Mac default is `--memory-mib auto`. The shared account uses the architectural
ceiling instead of a product resident quota, while the host adapter supplies
actual `hw.memsize` and Mach VM statistics for guest memory reports. `MemFree`
uses free pages; `MemAvailable` estimates free plus inactive pages, clamped to
physical total. Speculative pages already belong to free_count and are not
counted twice. This is an availability hint, not Linux's complete heuristic or
a physical reservation. Mach send rights are released after each query. Reports
are queried for proc/sysinfo metadata, not ordinary guest memory operations.
Numeric caps retain guest-budget reporting; data/RSS/address limits are unlimited
only in host-managed mode. Proc address-space limits follow getrlimit too.
The existing pressure/reclaim/pager path remains, as does the independent 16 GiB
per-mm dense mapping-descriptor ceiling and conservative fork accounting.

The EL1 restore prelude performs pending translation maintenance before loading
EL0 state. A single leaf change uses ASID + VA `VAE1IS`; root/intermediate-table
withdrawal or multiple different leaves use full-ASID `ASIDE1IS`. Both retain the
original DSB ISHST / TLBI / DSB ISH / ISB sequence. The completion marker clears
only after that sequence, so interruption before completion retries maintenance;
interruption during restore retains the authoritative complete input state.
No separate TLBI-only hypervisor entry/HVC is needed. The
[Linux ARM64 TLB implementation](https://github.com/torvalds/linux/blob/v6.12/arch/arm64/include/asm/tlbflush.h)
provides the matching VA/ASID operand and all-level invalidation reference.

Signed acceptance covers cold single-page faults without extra entries or
full-ASID flushes, interrupted VA/full-ASID prelude phases, complete GPR/SIMD
restoration, root reuse, permissions, unmap, table reclamation and code stores.
Hardware admission also persistently disables unused coordinator software CPU
aliases while preserving cached-byte-pointer invalidation and true fork COW.
The iPad software alias path and default memory policy remain unchanged.


## Immediate demand-fault continuation

An EL1 translation fault saves the complete task state in its private context.
Its Rust handler only edits memory and page tables. The next iteration in that
same execution slice therefore reuses the paused EL1 context and original timer
deadline instead of copying input state, resetting PSTATE and programming two
timer system registers again. The private handoff must have produced a memory
fault; task changes, yielded slices and stage-two exits use the full input path.
The timer is unmasked for continuation even if interrupted-adapter completion
masked it. Time in the host fault handler still counts against the slice. Every
exit retains a complete materialized common CPU state.

Fresh invalid-to-valid descriptors require publication, not TLB invalidation:
translation faults cannot be cached. The restore prelude uses DSB ISHST / ISB for
this case, including newly created intermediate tables. Valid mapping edits,
removals and root/ASID reuse retain their required invalidation; publication
never downgrades pending VA/full-ASID work. See Arm's
[memory management guide, section 8](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Learn%20the%20Architecture/LearnTheArchitecture-MemoryManagement-101811_0100_00_en.pdf?revision=1fdc3375-d81c-4457-b786-04fb98557de0)
and the [Trusted Firmware-A design](https://trustedfirmware-a.readthedocs.io/en/latest/components/xlat-tables-lib-v2-design.html).
There is no eager guest population, new resident charge or ordinary-memory fence.
Signed tests cover fresh leaf and all table-branch levels without TLBI, pending
publication interrupted before restore, register preservation through 64 new
faults, bounded timer programming, permissions, address reuse and root switching.


## Memory-model headroom review (2026-10-29)

The next high-value change is a common sparse VMA/resident-extent model plus a
Mac native anonymous backing provider. This keeps the shared userspace Linux
kernel and HVF execution. New measurements point to object/snapshot construction
and first-touch mediation rather than steady-state instruction execution in the
memory loop. These are research probes; the installed product path is unchanged.

### Untouched mappings still have substantial host metadata cost

A release probe using the pinned common runtime (87a4f20), hardware admission and
registry, thin LTO and product overflow checks maps accessible anonymous memory,
creates a View, commits, changes RW to R, creates another View, then unmaps. It
never reads/writes guest data and asserts zero guest resident bytes. There is no
vCPU. Two fresh processes per size/mode reverse variant order. For a 1 GiB VMA:

| Stage | Typical elapsed | Process peak RSS above baseline |
| --- | ---: | ---: |
| Create per-page mappings | 20.6 ms | 50.5 MiB |
| First hardware snapshot | 41.6 ms | 168.8 MiB |
| Snapshot after permission change | 84.3 ms | 283.2 MiB |
| Unmap | 43.2 ms | high-water mark retained |

`ru_maxrss` measures process peak RSS in Darwin bytes, not live RSS after release
or kernel-side page-table memory. Exact values depend on the build/allocator.
Native untouched mappings add about 0.02 MiB to this process metric; the existing
PROT_NONE reservation control adds about 0.2 MiB. Those controls are not drop-in
implementations of accessible guest memory. They demonstrate why range metadata
is worth pursuing. Removing the guest resident quota did not remove these host
objects. The independent 16 GiB descriptor guard should be replaced along with
the representation, rather than raised while this cost remains proportional to
all mapped pages.

### First-touch exits are a much larger target than timer setup

Five fresh processes per variant, seeded shuffled order, write/check one aligned
word per 4 KiB in 256 MiB. The native and hardware-only probes use the same
assembly store/stride/branch body. The hardware-only probe supplies a contiguous
anonymous host backing and preinstalled uniform 2 MiB stage-one blocks, then lets
HVF/macOS supply actual host pages on demand. It runs at EL1 with no Linux kernel
or userspace Linux runtime. Its one final HVC returns to the host.

| Variant | First-touch median | Same store loop after population |
| --- | ---: | ---: |
| Native assembly | 7.73 ms | 0.380 ms |
| Hardware-only lower bound | 16.24 ms | 0.398 ms |
| Full Trames Linux fixture | 97.28 ms | not measured with this store loop |

The native pthread version of the same C fixture used by Trames takes 7.96 ms
for first touch. All touched words/checksums match. Trames uses the unchanged
normal signed product, one CPU and 4096 MiB. The raw records retain every trial;
thermal state was nominal and low-power mode off before/after. No builds or
profilers ran during the measurements; ordinary desktop apps remained.

The approximately 6x ratio between Trames and the hardware-only first-touch
phase is headroom in this bounded case, not a total Geekbench/Firefox forecast.
The prototype omits Linux syscall dispatch, quotas, fork/COW, shared/file-backed
mappings, 4 KiB subrange changes, native leases and production invalidation. Its
steady-state store result establishes that this particular stable-mapping loop
is already close to native. The earlier original-workload profile found 97.6%
of faults exporting new storage and zero allocations equivalent to 96.1% of
faults, making private anonymous backing the appropriate first target.

### Recommended implementation sequence

1. Put virtual ownership and permissions in common VMA intervals. Keep resident
   storage/COW exceptions sparse. Views should share an immutable range index
   and changed extents, without constructing another Frame/ConcurrentPage graph
   for every untouched 4 KiB. Interpreter and native accesses must use the same
   permission, ownership and race semantics. Existing architectural range checks
   and configured resident quotas remain.
2. Add a Mac provider for stable native anonymous ranges with linear GPA/host-VA
   geometry. Build stage-one translations by range, using blocks for uniform
   regions and 4 KiB leaves where required. Physical population remains with the
   host pager. This removes the ordinary per-4-KiB Rust export/HVC cycle for the
   eligible range, rather than only shortening its handler. iPad uses the same
   VMA model with its software memory/execution provider; no second Linux kernel
   implementation is introduced.
3. On that foundation, publish non-destructive range additions without joining
   every worker of the mm. Permission withdrawal, unmap, COW and reuse still
   require the real affected translation/byte-lease boundary. Shared VMA state
   and per-range/object synchronization should replace derived snapshot work;
   do not add a coordinator thread, polling bus or ordinary byte barriers.

The initial native range scope should be host-managed private anonymous memory.
Strict resident caps retain fault admission until accounting is equivalent. A
partial mprotect/unmap must split translations correctly; Linux DONTNEED must
produce zeroes, unlike Darwin's content-preserving live-page reclamation hint.
Fork, retained native/GPU pointers and file/shared mappings must preserve their
existing semantics before entering the new path. This is a joint memory-model
change: sparse metadata addresses the untouched-mapping cost, while native
backing/range translations address first-touch exits. A contiguous canonical
backing can also help future native buffer access without per-page transport.

### Reproduction and validation boundary

Sources live in [benchmarks/memory-architecture](../../benchmarks/memory-architecture).
The Rust probe is a separate workspace and does not change product build members:

```sh
cargo run --manifest-path benchmarks/memory-architecture/Cargo.toml --release --locked -- dense 1024
cargo run --manifest-path benchmarks/memory-architecture/Cargo.toml --release --locked -- reserved 1024
cargo run --manifest-path benchmarks/memory-architecture/Cargo.toml --release --locked -- native 1024
mkdir -p target/memory-probes
clang -O2 -Wall -Wextra -Werror benchmarks/memory-architecture/touch-probe.c -framework Hypervisor -o target/memory-probes/touch-probe
codesign --force --sign - --entitlements apps/macos/Trames.entitlements target/memory-probes/touch-probe
target/memory-probes/touch-probe native 256
target/memory-probes/touch-probe hardware 256
```

The C fixture is compiled with `-O2 -pthread` on macOS, or statically with a Linux
AArch64 toolchain for the guest. Arguments `1 256 1 0 1` select this comparison.
The probe sources pass Rust format/release lint with warnings denied and checked
C compilation; native/HVF/fixture controls validate their values. No production
CPU, Linux, network or graphics source changed, so prior passing acceptance was
reused. At this research-probe checkpoint Firefox stayed closed and no new
Geekbench score was claimed. Full raw
records, compiler/source/binary identities and limitations are in
[the filtered evidence](../validation/mac-memory-model-probes.json).


### Production sparse VMAs and persistent canonical storage (2026-10-29)

Runtime `b230398dd102433599268a2fef4630d77a0a600c` implements the common sparse VMA portion of the
review above, shared with the iPad interpreter. Accessible private anonymous
mappings larger than 64 KiB keep interval metadata until reached. Frozen worker
views share canonical byte stores and reuse unchanged frame caches across normal
execution handoffs. They avoid constructing a second coordinator descriptor
graph after every worker slice. Fork/COW, permission changes, pins, quotas and
4 KiB subranges retain their existing checks and ownership boundaries.

Retirement tickets retain exactly the affected old stores and address intervals.
Frozen snapshot indexes update only overlapping range records and preserve their
unchanged frame caches. Mmap search and mprotect avoid complete process-region
lists. A 2,000-VMA regression bounds local release to its target store/cache,
at most two rebuilt range records, and zero complete region-list constructions.
Deferred retirement survives retained view generations; their shared dirty bit
keeps late canonical writes visible. Visited hardware chunks retain precise
stamps, preserving bounded revalidation and resident-neighbor reuse after CPU
migration. The common implementation and lifetime rules are documented in
[region memory](https://github.com/kb5000/trames-runtime/blob/b230398dd102433599268a2fef4630d77a0a600c/docs/architecture/region-memory.md#sparse-accessible-anonymous-ranges-2026-10-29).

Five fresh processes per variant, in shuffled order, measured an untouched
1 GiB RW mapping with no guest data touches:

| Stage | Before median | After median | Additional peak RSS, before / after |
| --- | ---: | ---: | ---: |
| Map | 20.686 ms | 2.42 us | 50.58 / 0.11 MiB |
| Initial freeze | 37.240 ms | 5.25 us | 180.95 / 0.19 MiB |
| Protect + refreeze | 108.607 ms | 10.08 us | 295.31 / 0.27 MiB |

RSS is `ru_maxrss` relative to each fresh process's initial peak, including earlier
stages in that process. A separate allocator-instrumented test bounds total
allocation requests for map/freeze/partial-protect below 128 KiB.

The unchanged static Linux pthread fixture also checked every reached value.
Both variants used normal signed binaries, a 4 GiB guest resident cap, no
windows/network, and five fresh shuffled repetitions per configuration:

| Workload | Before wall median | After wall median | Wall-time reduction |
| --- | ---: | ---: | ---: |
| 1 CPU, 256 MiB, one word per 4 KiB, one recreate round | 174.433 ms | 169.404 ms | 2.9% |
| 12 CPUs, 128 MiB each, full 8-byte stores/reads, two recreate rounds | 2233.883 ms | 2189.918 ms | 2.0% |

These are bounded memory-workload results. Hardware first touch still enters the
existing 4 KiB export/fault path; native range backing and block translations
remain the next larger execution-memory change. The Linux CI VM was stopped and
no builds, profilers or other test runners ran during measurements. Thermal state
was nominal and low-power mode was off before and after.

Validation passed the common macOS and native ARM64 Linux workflows, the Mac
workflow plus 32 signed HVF tests and the pressure test, and the iPad workspace
and both iOS compile targets. English evidence, binary hashes and raw bounded
measurements are in [mac-memory-model-probes.json](../validation/mac-memory-model-probes.json).

The separately completed clean Geekbench 6.7.1 Preview run used the same Linux
benchmark bytes, twelve slots and explicit 4096 MiB cap as the preceding control.
It scored **3678 single / 9134 multi**, versus **3388 / 8181** before (+8.6% /
+11.6%), and completed all 32 workloads in 275.91 s. Navigation multi rose from
2248 to 7522; Asset Compression from 13937 to 16907. Photo Library and several
image components did not improve, and Text Processing still scales poorly.
Both revisions have one clean full-run sample, so these are observed scores,
not confidence intervals or a claim that every workload improved. The public
[new result](https://browser.geekbench.com/v6/cpu/19404127) and
[preceding control](https://browser.geekbench.com/v6/cpu/19403147) are also recorded
with all component scores in [mac-geekbench.json](../validation/mac-geekbench.json).
The new totals are 85.8% / 49.1% of the earlier native macOS run; OS, binary,
library/ISA dispatch and reported 4/24 GiB memory differ. Firefox remained closed.


### Native anonymous ranges and live mmap publication (2026-10-29)

Runtime `68552f7bd3b9534fe15fe1fd4a5c4e22b7415998` and Mac source
`4ab04b4f90b9e391d0dca4eb710f6d23d90d2f5a` implement the next two mechanisms above.
Eligible host-managed private anonymous RW extents retain one stable demand-zero 2
MiB host allocation and use an L2 block translation. Registration does not touch
data pages; macOS supplies physical pages as hardware reaches them. The retained
signed acceptance checks all 512 guest pages with one data translation fault and no
individual data-page exports. Host virtual/GPA continuity does not require
contiguous physical host RAM. Later interpreter/native access uses the same common
owner, permissions and existing region/hardware-borrow gate.

Pure new anonymous RW mmap requests reuse Linux validation/address allocation on the
existing coordinator service queue. The caller releases its execution slot; peers
keep running. An insertion-only shared range index publishes the new ownership to
old Views after a cold lookup miss, retaining stable Frame addresses. No new service
thread, polling bus or normal byte barrier is introduced. Destructive generations
rotate the index owner. Permission withdrawal, unmap, discard, fixed/file/shared
mappings, traced and seccomp paths still join workers. A real interpreter CLONE_VM
regression reads the new mapping from a running peer's original View with zero VM
checkpoints and one service.

The host-managed account reports provider residency rather than charging every
virtual byte. Sparse fork and account rebinding materialize only host granules with
possible logical contents. Mach page dispositions include compressed and paged-out
data; failed queries conservatively retain the complete-range fallback. After an old
VM is destroyed, reached subpages rebind through canonical per-page backing and
transfer their accounting; old coarse owners retire as legitimate references drain.
A retained fragment can still keep the complete 2 MiB host allocation, with physical
residency managed in 16 KiB host granules. This does not promise 4 KiB physical
reclamation within a live extent.

The retained bounded trials use five shuffled fresh processes per
variant/policy/workload and identical static Linux pthread fixture bytes and
validate every value/checksum. Times include mapping, touch, read and unmap, not
just first touch. Single CPU recreates once; twelve CPUs recreate twice. Concurrent
thread phase sums overlap and are not added to derive wall time.

| Memory policy / workload | Before median | After median | After / before |
| --- | ---: | ---: | ---: |
| Host-managed, 1 CPU, 256 MiB, one word per 4 KiB | 168.877 ms | 19.520 ms | 0.116 |
| Host-managed, 12 CPUs, 128 MiB each, full stores/reads | 2171.105 ms | 134.185 ms | 0.062 |
| Host-managed, 12 CPUs, 64 MiB each, one word per 4 KiB | 927.642 ms | 53.474 ms | 0.058 |
| Strict 4096 MiB, 1 CPU, same sparse workload | 167.916 ms | 170.494 ms | 1.015 |
| Strict 4096 MiB, 12 CPUs, same dense workload | 2271.118 ms | 2174.117 ms | 0.957 |
| Strict 4096 MiB, 12 CPUs, same sparse workload | 906.434 ms | 956.094 ms | 1.055 |

The host-managed lifecycle speedups are 8.7x / 16.2x / 17.3x. Strict resident quotas
and explicit resident working-set targets retain original granule/page admission,
and the strict control wall changes are +1.54% / -4.27% / +5.48%. File/shared
mappings and nonuniform permissions retain the fallback too. The bounded trials use
runtime `e5a2523` / Mac `73ebdb1`; their memory, mmap-addition and Mac HVF sources
are unchanged in the final revision, so this component evidence is reused. Exact
older runner identities remain in the raw records. These results do not predict the
same whole-program speedup. The strict controls show no stable benefit, with the
sparse twelve-slot case about 5.5% slower in these five trials.

The separate clean full Geekbench comparison uses `auto` on both revisions, twelve
CPUs and the same unmodified Linux ARM benchmark. Both advertise the host's 24 GiB.
The preceding implementation scores **3595 single / 9362 multi**; the native-range
implementation scores **3885 / 12307** (+8.1% / +31.5%). Complete wall time is
275.85 -> 250.30 s. Multi-core Navigation is 7538 -> 14928, Photo Library 6132 ->
9777 and Photo Filter 4093 -> 13544. Asset Compression is 16413 -> 15492 (-5.6%);
Text Processing remains weak at 3367 multi versus 3525 single. These are observed
component results, not a uniform-speedup claim. Both complete all 32 workloads and
upload: [same-policy control](https://browser.geekbench.com/v6/cpu/19405122), [new
implementation](https://browser.geekbench.com/v6/cpu/19405231). Each revision has
one clean full sample, so component variation and remaining scaling limits must be
assessed in [the complete component evidence](../validation/mac-geekbench.json). The
new totals are 90.6% / 66.1% of the earlier [native macOS
result](https://browser.geekbench.com/v6/cpu/19400588). The latter also reports 24
GiB but differs in OS, benchmark binary, library/ISA dispatch and date; this is a
platform comparison, not isolated HVF overhead.

A provisional complete-suite attempt was sampled and stopped when multi-core Photo
Library exceeded nine times the control stage time. About 71% of its main-thread
samples were host residency queries during proc refresh. The runtime now observes
each real address-space owner once per snapshot and reuses that value for
aggregate/per-thread rows. The one-per-mm fix alone was insufficient: its clean
intermediate scored 3826/10366 but Photo Library multi fell 6132 -> 1132. A
temporary proc-path-only diagnostic then recorded 801 missing
`/proc/sys/kernel/random/uuid` probes before being stopped. Actual VFS traversal now
projects observation dependencies: independent endpoints and missing kernel keys
skip task snapshots, maps and other metadata skip residency, and real memory
observations stay live. A 13-task regression checks zero residency callbacks for
UUID/cpuinfo/maps and fresh values on the next statm request.
Alias/cwd/dirfd/missing-process cases are covered. The requested UUID v4 endpoint is
also implemented using the existing host CSPRNG, read-only runtime pseudo nodes and
discoverable directories, following [Linux random sysctl
semantics](https://docs.kernel.org/admin-guide/sysctl/kernel.html#random). No stale
cross-request timer cache is used. That diagnostic run has no performance score and
is excluded.

The final normal signed runner SHA-256 is
`4b964e1da487ac2dd5d7c6bebf78bad531a6360a9c63b1f8312a0314fbfac683`. The final full
Geekbench run uses that exact bundled executable; diagnostics are compiled out. The
native Linux CI VM is stopped and no builds, samplers, other probes or window
captures run during measurements. Thermal state is nominal and low-power mode off
before/after. Firefox remains closed.

Local common macOS workspace tests, the full native ARM64 Linux workflow and product
workflow checks passed. Later memory/provider changes passed scoped final
format/lint, 148 memory lib tests plus six integration tests on macOS and native
ARM64 Linux, 39 signed HVF tests, the app pressure test, normal signed release and
both iOS targets. The final proc/UUID fix additionally passes 202 interpreter
process conformance cases (three ignored), 325 VFS tests on both macOS and native
ARM64 Linux, affected lint and both iOS targets. Unchanged
CPU/Linux/network/graphics acceptance is reused. Existing unrelated network and iPad
integration work is excluded. Source identities, raw bounded trials and validation
scopes are in
[mac-memory-model-probes.json](../validation/mac-memory-model-probes.json). Common
ownership rules are in [region
memory](https://github.com/kb5000/trames-runtime/blob/68552f7bd3b9534fe15fe1fd4a5c4e22b7415998/docs/architecture/region-memory.md#native-anonymous-extents-and-additive-publication-2026-10-29).


### Host-relative optimization targets (2026-10-29)

The latest twelve-slot host-managed run is 90.6% / 66.1% of the earlier native
macOS single/multi scores. Both report 24 GiB. These are one-sample platform
results with different Linux/macOS binaries, libraries and ISA dispatch, not an
isolated HVF overhead estimate. Single/multi workload sizes differ, so their
score ratios are comparative scaling signals rather than active-core counts.

| Workload | Single, Trames / native | Multi, Trames / native |
| --- | ---: | ---: |
| Object Detection | 3186 / 6297 (50.6%) | 10316 / 18010 (57.3%) |
| Photo Library | 3450 / 3717 (92.8%) | 9777 / 23344 (41.9%) |
| PDF Renderer | 4218 / 3968 (106.3%) | 8537 / 19319 (44.2%) |
| HTML5 Browser | 4514 / 4466 (101.1%) | 10946 / 21252 (51.5%) |
| Asset Compression | 3636 / 3737 (97.3%) | 15492 / 25436 (60.9%) |
| Text Processing | 3525 / 4123 (85.5%) | 3367 / 5545 (60.7%) |

SME is a concrete first capability investigation. Native Geekbench reports
SME-I8I32, SME-F32F32 and SME2; the guest backend intentionally does not expose
SME. The [official workload description](https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf)
identifies SME as an Object Detection acceleration path. The installed Hypervisor
headers expose SME register/state APIs; a small signed read-only query returned
HV_SUCCESS and maximum guest SVL 64 bytes, while the host SME2 sysctl returned 1.
No VM or SME guest instruction was executed by this query. This supports a
capability hypothesis, not a demonstrated speedup or proof of the selected
native workload kernel.

Before advertising SME, validate guest instructions, extended per-thread context,
thread migration and the [Linux syscall/signal/prctl ABI](https://docs.kernel.org/arch/arm64/sme.html).
Apple's [SME state API](https://developer.apple.com/documentation/hypervisor/hv_vcpu_get_sme_state(_:_:))
is thread-affine. Keep Mac execution state in the backend, allocate/save extended
state only where used, and expose optional common Linux ABI support without
changing iPad's default capability set.

The strongest broad scaling candidates are the remaining mapping checkpoints.
Photo Library, PDF and HTML5 have near-native single scores but only 42-52% of
native multi scores. Unmap/protect/discard/remap and validation-only madvise
currently enter the mm checkpoint path: views drain, memory commits, and views
are prepared again. Fresh profiles should separate guest CPU, host handling,
waits and commit/stop costs. Reduce unrelated snapshot/rebuild work and handle
nondestructive advice separately, while retaining actual permission/TLB withdrawal
and byte-lifetime synchronization. These observations do not prove that mapping
checkpoints dominate any current component; historical profiles predate the
latest native-range and proc changes.

Further candidates are range ownership for eligible file/shared mappings and
more precise partial-extent reclamation. Classify remaining faults and copy/map
costs before extending the anonymous-only fast path; preserve EOF, COW, code
freshness, native/GPU aliases and precise permissions. Metadata-only proc queries
also still build all tasks' argv/fd rows, so a single maps/status request should
be considered for target/field projection. Futex waits/wakes and many short
syscalls already run locally; inspect measured fallback/lock/wake costs instead
of assuming they all traverse the coordinator. Text Processing scales weakly on
the native platform too, making it a lower-confidence target than the three
strong multi-core gaps above.

The full per-workload ratios, capability-probe result and validation plan are
recorded under host_relative_review in [mac-geekbench.json](../validation/mac-geekbench.json).
This review changes documentation only; existing unchanged-source validation is
reused and no new performance score is claimed.
