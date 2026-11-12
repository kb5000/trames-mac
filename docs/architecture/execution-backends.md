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
state is preserved by the backend and common signal ABI. The baseline profile
covers FP/ASIMD, AES/PMULL/SHA/CRC, LSE, FP16, RDM, dot-product, FHM, I8MM and BF16.
A bounded SME/SME2 streaming profile is also available on supported HVF
configurations, as described below. CPUID, non-streaming SVE, SME FA64/FFR, PAC
and unrelated PSTATE state remain absent. Separate instruction and SME fixtures
check results; physical-host capabilities alone are not an admission oracle.

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
and live-process RSS observations before/after reclamation. Native-pager
deactivation does not promise an immediate RSS decrease; data/fork correctness
and actual withdrawal/page-in activity remain acceptance gates. Shared-map and code-cache fixtures
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
on vCPU retirement and thread teardown. There is no shared counter lock, monitor
thread or guest load/store instrumentation. Profiling changes timings; use a normal signed
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
write/read/execute faults, zero/copy allocations, imports, SME context exports,
SME streaming, worker-to-worker migration and worker/owner transitions. Compare
cumulative deltas over workload boundaries; sum only final per-thread records
for totals. Historical versions emitted nine or twenty counters.

`vcpu_exec_ns` uses the public ARM `hv_vcpu_get_exec_time` API on the creator
thread and converts its mach_absolute_time ticks through mach_timebase_info.
It includes EL0 and the thin EL1 adapter, not retired instructions; it is nested
inside HVF run time. `vcpu_exec_errors` counts SDK sampling failures. TLS tracks
live vCPU IDs/host pointers only between Executor creation and destruction and
accumulates retired totals before IDs can be reused. Other host threads never
sample those pointers. `registers` orders mailbox exports, fallback exports,
completed exception saves, HVF entries, completed invalidation batches, full-ASID
batches, interrupted restores, fault continuations, timer programming, descriptor
publications and cancellation requests. These are batch counts, not TLBI
instruction counts. `svc` contains sparse AArch64 SVC-entry pairs, not completed
syscall counts; `mprotect[8]` indexes the low three protection bits of argument 2.


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
SME-I8I32, SME-F32F32 and SME2; the reviewed baseline did not expose SME.
The implemented follow-up is recorded below. The [official workload description](https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf)
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


### Bounded hardware SME execution (2026-10-29)

This implementation supersedes the capability investigation above. HVF's actual
PFR/SMFR configuration and SME API must support the bounded 64-byte profile on
macOS 15.2 or newer before auxv/proc advertise SME. Optional arithmetic and SME2
bits follow those registers. On the tested Apple M6, requests for SMCR lengths
16 or 32 bytes still execute at 64 bytes; Linux prctl therefore reports the
only supported length, including requests below it. No full Linux kernel starts.

The common 816-byte AArch64 layout is unchanged. A task allocates an optional
6,352-byte extended buffer on its first SME/TPIDR2 access; ordinary tasks
have no matrix buffer. The existing 16 KiB per-vCPU private allocation now holds
the extended mailbox, using two EL1-only 4 KiB leaves in the existing adapter
reservation. EL1 saves/restores Z/P/ZA/ZT directly through this mailbox; ordinary
exception exits do not issue one HVF API call per extended register. Streaming
mode is stopped before NEON context operations, and low vector lanes are
preserved. Timer/stage-two exits use the framework's cold export path. Immediate
handled faults reuse the saved context and original timer without task-buffer
copying. The creator-thread owner excludes concurrent mailbox access; no Rust
slice spans hardware execution and no ordinary guest-memory barrier is added.

Optional common hooks implement Linux SME signal records, vector-length prctl,
fork/CLONE_VM and exec semantics. AT_MINSIGSTKSZ advertises 8192 bytes. iPad's
interpreter retains its original capabilities and storage model. Non-streaming
SVE, FA64/FFR and debugger SME regsets are not added in this change.

Validation includes actual FP32 FMOPA results, ZA/ZT/TPIDR2 preservation,
streaming-vector/predicate preemption, host-thread/vCPU migration, private
context access denial, malformed-frame rollback and extended altstack bounds.
The static Linux `linux-sme` fixture additionally verifies deferred/inherited
exec policy, fork, CLONE_VM and signal delivery in streaming mode with a
clobbering handler. Existing real-ELF signal recovery still matches its native
Linux oracle. Shared process conformance and both iOS target checks pass.

A fresh diagnostic suite attributes 57,151 mapping calls to 47,764 mm checkpoint
groups: 38,652 munmap, 18,253 mprotect, 149 destructive madvise, 41 advisory
madvise, 51 non-additive mmap and five brk calls. Three short multi-core samples
of HTML5/PDF/Photo Library contain substantial queue waits as well as guest
execution and per-page handling. These waits mix guest synchronization and
mapping pauses; they are not all removable overhead. Advisory madvise is a very
small candidate in this data. Reducing the withdrawal/protection path is the
next broad mechanism to investigate while preserving completed-call permission
and backing-lifetime guarantees.


Build `tests/fixtures/linux-sme.c` and `linux-sme.S` with a static Linux AArch64
musl toolchain and an assembler supporting SME2 (or the Mac Clang/musl sysroot
and rust-lld recipe used by the CoreMark fixtures). On a supported Mac, run:

```sh
python3 tests/integration/mac-sme.py \
  --runner target/release/trames-mac-run \
  --fixture /path/to/linux-sme \
  --output target/sme-linux-acceptance
```

The harness checks one bounded-memory CPU and twelve host-managed CPUs using
the identical ELF, with networking and windows disabled.


SME permission is lazy per task. Unused tasks keep SMEN/EnTP2 disabled and skip
the extended adapter. The first architecturally trapped SME or TPIDR2 access
allocates state, enables controls and retries that same instruction on the same
host thread. A fixed SMCR is configured only on this transition; an ISB is
required only when controls change. Switching back to an unused task clears the
previous streaming mode before disabling access. A regression alternates
ordinary tasks and TPIDR2-only tasks and checks both non-activation and isolation.
The initial eager candidate scored 4079/11520: Object Detection single improved
to 6560, but the multi total was below the previous 12307 sample. It is retained
as provisional evidence, not accepted as a general performance improvement.


The accepted installed run is [4127 / 12551](https://browser.geekbench.com/v6/cpu/19405427),
compared with a fresh frozen-binary control of
[3831 / 12206](https://browser.geekbench.com/v6/cpu/19405419): +7.7% single and
+2.8% multi. Object Detection is 3106 -> 6437 single and 10443 -> 13242 multi;
Background Blur is 3426 -> 4252 and 11285 -> 12729. The earlier native macOS
sample is [4287 / 18616](https://browser.geekbench.com/v6/cpu/19400588), putting
the new aggregate at 96.3% / 67.4%. Object Detection single is in the same range
as native's 6297. Different OS binaries/libraries prevent treating those native
ratios as pure executor overhead.

These are one clean sample per variant, not confidence intervals or a uniform
improvement: PDF multi is 9537 -> 8257, and Photo Filter is 12294 -> 11753.
PDF/Photo Library/HTML5 multi remain approximately 42.7%/43.8%/54.1% of native.
No builds, CI VM, diagnostics, metrics, samplers or competing benchmark ran
during the clean pair. Source revisions, exact installed runner hash, all
component scores, provisional eager candidate, profile counts and verification
scope are saved under `sme_followup_2026_10_29` in
[mac-geekbench.json](../validation/mac-geekbench.json).


### Interruptible mapping checkpoint batches (2026-10-29)

The original fixed-work probe attributed 252 ms of a 330 ms mapping phase to
waiting for related CPUs to return; memory commit, calls and rebuild together
cost under 2 ms. Merely requesting immediate `hv_vcpus_exit` reduced individual
waits but regressed total wall time from 330 to 521 ms: checkpoint groups rose
from 152 to 3,011 while peers repeatedly stopped and woke. Requeueing peers
first alone still took 479 ms. These prototypes are not accepted speedups.

The implementation combines targeted exits with bounded batching. One retained
handle is bound per running worker assignment to its actual mm/backing domain.
Publication and pause use the domain's object lock, covering both pre-entry
orders without locking guest execution. HVF latches an exit for a CPU not yet
running. A separate per-CPU lifetime lock makes retained handles inert before
CPU destruction/ID reuse. The existing 2 ms timer remains a fallback if an
optional exit request fails. Other address spaces continue executing, and native
or I/O operations return normally before the checkpoint takes ownership.

While already exclusive, the existing owner runs consecutive VM syscalls in a
250 microsecond window shared by all requesters in that checkpoint. This also
requires all admitted cohort jobs to be held in that checkpoint, reserving a
logical CPU slot without blocking unrelated address spaces. Partial checkpoints use their existing local mapping commit and worker execution. Each guest entry receives only the remaining hardware timer budget.
Task, aggregate and per-CPU time accounting include these owner CPU entries. It uses the same
canonical memory/view preparation, permission, byte-lifetime, ASID/TLB and SME
state path as coordinated execution. All other syscall, native and fault exits
remain pending on the original job and are handled exactly once by its ordinary
worker, or the joined kernel if cancellation wins the handoff. No native body
runs on the owner through this mechanism. Requeueing preempted peers before
mapping requesters preserves useful progress between windows. The common hook
is unsupported by default; iPad retains interpreter execution and capabilities.

`tests/fixtures/vm-checkpoint.c` performs 1,000 mmap/mprotect/mprotect/munmap
rounds beside finite fixed CPU work (200 million xorshift iterations per peer).
The final implementation has five shuffled old/new repetitions per CPU setting,
with no concurrent builds, producing these medians in milliseconds. All per-thread results match across
variants. This is a targeted synchronization probe, not an application score.

| CPUs / compute peers | Old mapping | New mapping | Old total wall | New total wall |
| --- | ---: | ---: | ---: | ---: |
| 1 / 0 | 17.704 | 17.675 | 17.707 | 17.678 |
| 6 / 5 | 309.238 | 20.000 | 309.302 | 291.309 |
| 12 / 11 | 313.443 | 37.868 | 313.562 | 281.984 |

The 6/12 CPU total improves about 5.8%/10.1%; no single-CPU improvement is
established. Five observations are not confidence intervals. The new 12-CPU
mapping samples range from 36.709 to 53.100 ms and total wall from
280.637 to 299.651 ms. Retain full samples and hashes rather than presenting only the best run.
Compile the fixture with a static AArch64 Linux musl toolchain (`-O2 -pthread
-ffixed-x18`), then run it headless with identical binaries and parameters. Timed probes omit `--metrics`; enable it separately for phase
diagnostics:

```sh
trames-mac-run --no-pty --no-windows --network off --memory-mib auto --cpus 12 \
  /path/to/vm-checkpoint 11 1000 200000000
```

`--metrics` reports VM phase microseconds in this order: quiescence, owner queue,
commit, Linux calls, rebuild, resume, exclusive CPU. Wait bins are below
5/20/100/500/1000/2000/5000 microseconds, then at least 5000. Batch counters are
used windows, exclusive CPU entries and additional coalesced VM calls. These
are phase diagnostics; overlapping domain times do not sum to session wall time.

A 16 microsecond per-entry probe cap was also tested and rejected. It reduced
exclusive CPU time from 836 to 495 ms in one full suite, but checkpoint groups
rose from 7,182 to 12,529 and quiescence time from 206 to 595 ms. The full score
was 4104/12712 versus 4131/13020 for the 250 microsecond shared-window prototype;
PDF multi was 9070 versus 11919. These are one sample per variant, not a confidence
interval. A shorter speculative slice did not reduce the measured complete
mechanism cost. The final implementation retains the single shared 250
microsecond deadline plus the whole-cohort capacity guard and full CPU accounting.


The final installed release binary (`0757afcb6e6943ec8e90bd0f2356c98ff1065b6340e2b0c10c97c950573b3e0b`)
uses Mac code `8c0eb00b896f9976d85c70d1dfeb032a6c4d408f` and runtime
`d21663eb30be8d550c0abdad1e6adaefc0bb49fc`. Fresh complete 12-CPU, host-managed
Geekbench 6.7.1 Linux runs (both with `--metrics`, without the HVF diagnostic
feature) score
[4161/12329 for the frozen control](https://browser.geekbench.com/v6/cpu/19405575)
and [4136/12881 for the final build](https://browser.geekbench.com/v6/cpu/19405649):
single -0.6%, multi +4.5%. PDF multi rises from 7682 to 11651 (+51.7%).
HTML5 multi is 10844 versus 11675 (-7.1%); Photo Library remains essentially
unchanged. This is one complete sample per variant, not a confidence interval,
and does not establish a uniform workload improvement. Earlier experimental
scores are retained separately from the final capacity/accounting-correct build.

The final suite coalesces 55,315 additional mapping calls. Quiescence costs
194 ms, Linux calls 1501 ms and exclusive CPU 810 ms across the complete suite;
phase durations can overlap and do not explain all remaining application time.
Full per-workload scores, fixed-work repetitions, binary hashes, rejected
experiments and scoped validation are in `vm_checkpoint_followup_2026_10_29`
in [mac-geekbench.json](../validation/mac-geekbench.json). Paging acceptance now
retains RSS/footprint as observations: its former immediate 24 MiB RSS-reduction
assertion also failed on the frozen old runner under the existing native pager.
Complete bytes, shared atomics, fork/COW and actual withdrawal/page-in activity
remain gates; no production memory/pager behavior changed in that correction.


### Parallel metadata, directed wakeups and clock vDSO (2026-10-29)

The common runtime now reuses frame/chunk metadata within each frozen worker
View and notifies eligible sleeping workers individually. Dynamic residency and
backing/code versions are still read live; missing and first-touch chunk stamps
are not cached. Initial placement, affinity, outstanding I/O/deadlines and stop
retain blocking wakeup coverage. Ordinary guest/native memory and algorithms
gain no lock, barrier, polling bus or resource thread. Both products use these
common changes; the iPad interpreter retains its execution/capability defaults.

Mac enables the common AArch64 Linux vDSO for monotonic clock queries. The
public Hypervisor SDK defines `CNTVCT_EL0 = mach_absolute_time() - vtimer_offset`.
Each vCPU explicitly uses offset zero, and the host validates CNTFRQ against
`mach_timebase_info` before advertising the counter. The preemption timer uses
the same documented Mach domain. Initialization/exec calibrate against the
existing session/Wayland origin while stopped, with at most eight attempts and
a one-microsecond bracket limit. Failed calibration uses the syscall. Guest
counter reads do not exit HVF or rendezvous with the owner. Realtime still uses
the host syscall path, so wall-clock changes need no updater thread. The common
vDSO retains existing clock-ID semantics; no new host-suspend policy is claimed.

Fixed-work probes use five shuffled repetitions per variant/CPU setting, with
no concurrent compiler or sampler. They isolate the changed mechanisms and are
not application speedups or confidence intervals:

| Probe | Configuration | Old median | New median |
| --- | --- | ---: | ---: |
| Metadata queries | 1 thread, 200k iterations | 6.824 ms | 3.424 ms |
| Metadata queries | 6 threads, 200k each | 127.874 ms | 23.191 ms |
| Metadata queries | 12 threads, 200k each | 382.359 ms | 74.767 ms |
| Futex token ring | 1 CPU, 2 threads, 1k turns each | 2.639 ms | 2.625 ms |
| Futex token ring | 6 CPUs, 6 threads, 1k turns each | 35.059 ms | 33.799 ms |
| Futex token ring | 12 CPUs, 12 threads, 1k turns each | 169.582 ms | 82.139 ms |
| libc monotonic clock | 1 CPU, one caller, 200k calls | 1274.0 ns/call | 15.0 ns/call |
| libc monotonic clock | 12 CPUs, one caller, 200k calls | 1274.1 ns/call | 15.2 ns/call |

Metadata checksums match and remain at zero guest resident bytes. The ring
isolates directed wakeups using controls that both already have metadata/vDSO
changes. Clock timing uses one caller even in the 12-CPU configuration; separate
12-thread tests check monotonicity and syscall agreement. The initial metadata
run that overlapped compilation is excluded. Diagnostic queue-wait samples mix
guest synchronization, idle workers and runtime coordination; they cannot all
be labeled removable overhead or converted into useful guest core occupancy.
Darwin `proc_pid_rusage` CPU times use Mach ticks, requiring the measured 125/3
timebase conversion on this host.

Build `tests/fixtures/linux-clock.c` and `tests/fixtures/futex-ring.c` unchanged
with a static AArch64 Linux musl toolchain (`-O2 -static -pthread -ffixed-x18`).
The source-bound acceptance harness checks libc/versioned symbols, syscall
agreement, concurrent clocks, fork/exec, syscall-count reduction and futex
handoffs without imposing a timing threshold:

```sh
python3 tests/integration/mac-parallel-clock.py \
  --runner target/release/trames-mac-run \
  --clock /path/to/linux-clock --ring /path/to/futex-ring \
  --output target/parallel-clock-acceptance
```

Actual signed HVF tests cover long elapsed counter conversion, nanosecond carry
and realtime fallback. Shared memory/session/Linux regressions, Mac Clippy/unit
checks, both iOS embedding targets and existing real Linux SME, signal and SMP
paging fixtures pass. No actual macOS suspend/resume test was performed; the
counter-domain behavior relies on the public SDK contract, rather than an
unverified private commpage layout or periodic synchronization thread.


The first candidate failed before Geekbench main in glibc's vDSO registration.
Its two `PT_LOAD` headers triggered the older loader's zero-address sentinel,
producing a one-page load-bias error. The accepted layout describes only the
code in the ELF load segment; calibration lives in its adjacent R mapping.
`vdso-loader.S` is a freestanding dynamic PIE using the real glibc interpreter.
It fails with SIGSEGV on the saved two-load candidate and reaches its success
marker after the layout fix. Geekbench dynamic startup also passes. No failed
startup is counted as a benchmark result. This extends coverage beyond the
initial static-musl tests, which could not expose the dynamic-loader defect.

Link this probe with an AArch64 Linux linker using `-pie -e _start` and
`--dynamic-linker=/lib/ld-linux-aarch64.so.1`. Add these arguments to the harness
above to exercise the actual supplied glibc loader (no libc dependency is
required by the probe):

```sh
  --dynamic-loader-probe /path/to/vdso-loader \
  --glibc-loader /path/to/ld-linux-aarch64.so.1 \
  --rootfs /path/to/Alpine.tar.gz
```


The final normal signed CLI/app has SHA-256
`846b2b8500f489ccee246e6da38e7eef66a80ebb151ebc8b5a6b31d17c79b27e`, Mac
code `e4000c18593ea411424834250a50ef5af007af5d` and runtime
`8fb524ec5212f7d7dd5783ebcbe93543d702a6e9`. Four complete Geekbench 6.7.1
Preview Linux runs used old/new/new/old order, 12 CPUs and host-managed memory,
with metrics enabled and HVF diagnostics disabled. No compiler, CI job or sampler
ran during them; thermal state stayed normal and low-power mode was off.

Frozen controls are [4110/12836](https://browser.geekbench.com/v6/cpu/19405938)
and [4075/12743](https://browser.geekbench.com/v6/cpu/19406037). The installed
version is [4174/13020](https://browser.geekbench.com/v6/cpu/19405988) and
[4115/13007](https://browser.geekbench.com/v6/cpu/19406023). Arithmetic means
move from 4092.5/12789.5 to 4144.5/13013.5: +1.3% single and +1.8% multi.
These are two samples per variant, not confidence intervals. Relative to the
earlier [native macOS 4287/18616](https://browser.geekbench.com/v6/cpu/19400588)
sample, the new means are 96.7%/69.9%; OS binaries/libraries differ.

PDF and Text Processing multi improve about 12.7%/12.6%. HTML5 is essentially
flat. Photo Library and Object Detection multi regress about 4.0%/5.2%; the
metadata and handoff microbenchmarks do not imply a uniform application gain.
Photo Library and HTML5 still show the largest gaps to the earlier native
sample. Remaining executor cost must be separated from platform/library work;
no claim is made that all of those gaps are runtime overhead or mapping waits.

The table compares multi-core scores. Control/new columns are means of the two
fresh samples; previous published and native columns are their earlier single
samples. Full single-core/component results, every raw observation and hashes
are in `parallel_clock_followup_2026_10_29` in
[mac-geekbench.json](../validation/mac-geekbench.json).

| Workload (multi) | Previous published | Fresh control mean | New mean | Change | Native macOS |
| --- | ---: | ---: | ---: | ---: | ---: |
| File Compression | 9030 | 9086.0 | 9164.0 | +0.9% | 12298 |
| Navigation | 13478 | 14120.0 | 14154.5 | +0.2% | 16886 |
| HTML5 Browser | 10844 | 11144.0 | 11197.5 | +0.5% | 21252 |
| PDF Renderer | 11651 | 10764.0 | 12133.5 | +12.7% | 19319 |
| Photo Library | 10425 | 10428.0 | 10014.0 | -4.0% | 23344 |
| Clang | 24603 | 24766.0 | 24774.0 | +0.0% | 29922 |
| Text Processing | 3216 | 3297.0 | 3713.5 | +12.6% | 5545 |
| Asset Compression | 17996 | 17577.0 | 17804.5 | +1.3% | 25436 |
| Object Detection | 12298 | 12533.0 | 11883.0 | -5.2% | 18010 |
| Background Blur | 12960 | 12830.5 | 13579.0 | +5.8% | 18671 |
| Horizon Detection | 20524 | 20452.5 | 19847.0 | -3.0% | 25922 |
| Object Remover | 15063 | 15683.0 | 15707.5 | +0.2% | 20020 |
| HDR | 17879 | 18291.5 | 18367.5 | +0.4% | 23779 |
| Photo Filter | 13508 | 11566.5 | 12531.0 | +8.3% | 14494 |
| Ray Tracer | 22618 | 21554.0 | 21743.0 | +0.9% | 26110 |
| Structure from Motion | 16274 | 15817.5 | 15503.5 | -2.0% | 20364 |

The final installed binary repeats all five shuffled clock/ring observations
shown above. Its libc clock counter runs record four total syscalls for 200,000
monotonic reads, versus 200,006 on the frozen old binary. The futex control uses
the earlier broadcast queue while retaining metadata/vDSO changes; the final
layout-only ELF fix also applies to the installed side. All checksums match.
The shared metadata probe remains zero-resident and is independent of ELF
layout. No timing gate is added to correctness tests.


## HTML5 investigation and rejected translation prototype (2026-10-29)

Geekbench 6 HTML5 Browser exercises a headless CPU rendering stack: Gumbo,
litehtml, FreeType, AGG, JPEG and PNG. It renders eight pages in the single-core
workload and 32 in multi-core. This experiment therefore runs without a Trames
window or Wayland presenter. See the
[official benchmark internals](https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf).
Native macOS and Linux benchmark binaries use different OS/library stacks; the
remaining score gap is not a direct measurement of executor overhead alone.

The first diagnostic interval inside multi-core HTML5 contains 75,385 new-storage
faults, 10,831 retained-storage faults, 683,729 retained-neighbor PTE installs,
2,260 worker migrations and 7,706 mmap/munmap SVC entries. Full-ASID invalidations
account for 2,438 of 2,454 completed invalidation batches. These are partial
per-thread first-to-last cumulative deltas inside the phase, not identical
workload boundaries or exact wall-time percentages. The phase includes the
benchmark's intentional approximately five-second pause. Fault timers contain
nested allocation/map work, and SDK vCPU time includes the thin EL1 adapter;
neither may be summed as an independent elapsed-time fraction.

A bounded worker-locality prototype preferred the last worker for seven claims
before a FIFO claim, retaining affinity and idle stealing. Correctness passed,
but five shuffled fixed-work observations and a complete Geekbench run did not
establish an HTML5 improvement (HTML5 multi 11,338). It was removed from the common
runtime and both product copies. No queue policy change is included here.

A second Mac-only prototype batched up to 64 distinct ASID/VA operands in
the existing EL1 restore prelude. Multiple leaf edits, empty ancestor-table
withdrawal and valid 2MiB block edits used all-level VAE1IS; root changes, ASID
reuse and list overflow retained full-ASID ASIDE1IS. It preserved the translation
barriers and interrupted-restore protocol and fit the existing private 16KiB
context allocation. This prototype also was rejected and removed. Production
retains the previous VA/full-ASID selection, original C/assembly/hardware tests
and exact previously validated signed release executable. There is no common
Linux/iPad change or added RAM reservation.

A diagnostic interval of the rejected implementation records 76 full-ASID batches
out of 1,947, versus 2,438 out of 2,454 before (about 4% versus 99%). Its 74,125
new-storage faults and 697,769 neighbor installs remain large. Each interval has
different per-thread endpoints; the counts demonstrate changed invalidation
selection, not a 97% reduction in total execution time or equal-work timings.
Both captures report zero SDK execution-time errors. Each has one incomplete
JSON line from terminating the diagnostic process after HTML5; that line is
excluded explicitly.

| Measure | Current mean | Rejected VA batch mean | Change | Native macOS |
| --- | ---: | ---: | ---: | ---: |
| Total single | 4140.5 | 4164.0 | +0.6% | 4287 |
| Total multi | 12979.5 | 12948.5 | -0.2% | 18616 |
| HTML5 Browser single | 4454.0 | 4393.0 | -1.4% | 4466 |
| HTML5 Browser multi | 12484.5 | 11574.5 | -7.3% | 21252 |
| PDF Renderer multi | 12493.5 | 11334.5 | -9.3% | 19319 |

The comparison uses two normal, signed release samples per variant in
old/new/new/old order, 12 CPUs and host-managed memory. Metrics are enabled,
HVF diagnostics disabled, with no concurrent build, CI or sampler. Every run
starts and ends at thermal state zero with low-power mode off. The second new
sample uses the normally installed bundle after source formatting/comment
cleanup; reversing those exact edits reproduces the tested prototype's six
source hashes. Raw component results, binary/source hashes, public result links,
profile windows and scoped validation are recorded in
`html5_followup_2026_10_29` in [mac-geekbench.json](../validation/mac-geekbench.json).
Two observations per variant do not establish confidence intervals or uniform
gains; individual workload regressions are retained in that record. One additional
installed run completed all workloads but could not upload because the restricted
sandbox denied DNS resolution. It produced no score and is recorded separately;
only that failed sample was rerun with the previously authorized network access.

The lower two-sample HTML5/PDF means and flat aggregate do not justify shipping
this complexity. Fewer full-ASID invalidations did not translate to a repeatable
application gain. The native single-core HTML5 comparison is close; the current
multi-core mean is about 59% of the earlier native sample. The most useful next
experiments are reducing repeated per-vCPU page-table population, attributing
mmap/munmap quiescence and stage-two registration churn, and batching retained
metadata/PTE preparation. Shared per-mm page tables need separate proof of
cross-vCPU revocation, ASID/context lifetime, COW and reclaim. None is implemented
or claimed faster here; unused mappings must keep their demand-allocation policy.

Only the opt-in diagnostics are retained: public SDK vCPU execution accounting,
register-maintenance counters, worker/owner transitions, SVC-entry histograms and
mprotect permission-bit counts. Default builds compile out all new state and
callsites. The current signed production executable is restored byte-for-byte.

The rejected TLBI prototype passed 50 signed HVF tests and the application
pressure test,
including batched permission revocation, last-leaf table withdrawal/GPA reuse,
capacity overflow, interrupted list invalidation, block demotion, fork/COW and
paging. Its ordinary four HVF and seven CLI tests, eight real-Linux
clock/futex/glibc-loader cases, two SME cases, four signal oracles and three
SMP/paging cases passed too. After restoring the original production adapter,
Clippy with/without diagnostics passes again; a signed diagnostic build runs the
real Linux threaded clock/fork/exec fixture with 12 reports and zero SDK errors.
No common runtime changes are included; both product pins remain at 8fb524e, so
previous unchanged shared-runtime/iOS checks are reused. The restored production
binary retains the already completed baseline checks and clean benchmark runs.


## Translation reuse and pause-frequency experiments (2026-10-29)

The next experiments target the two mechanisms identified above: repeated
per-vCPU translation population, and pauses around mapping changes. Three
prototypes were implemented and measured, then removed. The installed signed
CLI/app remains byte-for-byte at SHA-256
`846b2b8500f489ccee246e6da38e7eef66a80ebb151ebc8b5a6b31d17c79b27e`;
the default exclusive window remains 250 microseconds and both product runtime
pins remain `8fb524ec5212f7d7dd5783ebcbe93543d702a6e9`.

The translation prototype shares immutable hardware L3 tables across vCPUs.
Each vCPU retains private upper tables, ASID and EL1 context, and copies a shared
leaf table before editing it. Import validates current canonical storage IDs,
GPAs and permission bits, and creates current view write tokens. Cached templates
hold weak guest-storage references, so they do not keep guest RAM resident.
Refcount synchronization protects actual table ownership; it does not strengthen
ordinary guest memory ordering or serialize guest algorithms. Cross-thread
execution, producer destruction, permission revocation on both CPUs, VA/GPA reuse,
COW and release tests pass: 49 signed HVF tests plus one application pressure
test, with workspace/all-targets Clippy also passing.

This tests actual cross-vCPU hardware leaf reuse, but retains per-vCPU upper
tables and validation/preparation work. It does **not** implement a fully shared,
mutable per-mm root and metadata structure. Its outcome does not rule out that
larger architecture; that architecture remains unmeasured.

Six complete normal Geekbench runs use A/B/C/C/B/A order: current baseline,
immutable L3 sharing, and a 1ms exclusive window. All use 12 CPUs, host-managed
RAM, metrics enabled and HVF diagnostics disabled, without a window, CI VM,
compiler, test or sampler running concurrently. Thermal state is zero and
low-power mode off before and after every run. These are arithmetic means of
two observations per variant, not confidence intervals. All 16 component scores,
hashes and public links are retained under
`translation_pause_followup_2026_10_29` in
[mac-geekbench.json](../validation/mac-geekbench.json).

| Measure | Current 250us | Shared immutable L3 | Change | 1000us window | Change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Total single | 4146.5 | 4105.0 | -1.0% | 4105.5 | -1.0% |
| Total multi | 13099.5 | 13107.0 | +0.1% | 13074.0 | -0.2% |
| HTML5 Browser single | 4458.0 | 4380.5 | -1.7% | 4459.0 | +0.0% |
| HTML5 Browser multi | 11072.5 | 11679.0 | +5.5% | 11267.0 | +1.8% |
| PDF Renderer multi | 11707.0 | 12446.0 | +6.3% | 13283.5 | +13.5% |
| Photo Library multi | 10163.5 | 11378.0 | +12.0% | 10192.0 | +0.3% |
| Object Detection multi | 13084.5 | 12236.5 | -6.5% | 12377.0 | -5.4% |
| HDR multi | 18707.5 | 16738.0 | -10.5% | 17371.0 | -7.1% |

Public baseline runs are [4165/13109](https://browser.geekbench.com/v6/cpu/19406562)
and [4128/13090](https://browser.geekbench.com/v6/cpu/19406643); shared L3 runs are
[4137/13068](https://browser.geekbench.com/v6/cpu/19406583) and
[4073/13146](https://browser.geekbench.com/v6/cpu/19406630); 1ms runs are
[4092/12904](https://browser.geekbench.com/v6/cpu/19406600) and
[4119/13244](https://browser.geekbench.com/v6/cpu/19406617). There is a measured
HTML5/Photo Library improvement in the shared-table means, but a flat aggregate
and lower means elsewhere. These samples do not justify enabling that added
ownership/validation machinery by default.

Fixed workloads isolate the mapping path. Each case has five shuffled normal
release observations at 1/6/12 CPUs, with exact checksums and no timing threshold.
`memory-paths.c reuse` prewarms a read-only VMA in the parent, then runs 2x CPU
threads with a 4KiB stride. This excludes the existing uniform RW 2MiB-block
path. At 12 CPUs, shared L3 reduces median work from 7.964ms to 7.738ms (-2.8%),
while full process time rises from 37.981ms to 38.557ms (+1.5%). A narrower work
timer alone would overstate the benefit.

`churn` uses one thread per CPU, each performing 60 disjoint mmap/touch/verify/
munmap cycles of 64 pages. Shortening the existing exclusive window increases
checkpoint frequency. In one 12-CPU comparison, 250us/32us/0us produce median
261/642/642 checkpoint groups, 14.437/34.426/33.662ms quiescence and
106.051/139.631/135.633ms work. Removing the window makes this workload slower.
In a separate matched comparison, extending it to 500us/1000us reduces checkpoint
groups and improves mapping throughput:

| Fixed workload at 12 CPUs (median) | Current 250us | 500us | 1000us |
| --- | ---: | ---: | ---: |
| Churn work, ms | 107.407 | 91.879 | 86.095 |
| Churn checkpoint groups | 256 | 149 | 92 |
| Churn quiescence, ms | 15.364 | 8.455 | 5.699 |
| Mapping beside compute peers, ms | 38.437 | 29.808 | 26.125 |
| Same compute-peer workload wall time, ms | 288.147 | 279.881 | 276.489 |

The 1ms window improves churn work by 19.8% and the concurrent workload's mapping
portion by 32.0%, but its whole-workload wall time improves only 4.0%. The shared
deadline bounds additional guest execution while peers are paused; mapping
syscalls and setup can take longer. Full Geekbench aggregate performance is flat,
so the production window stays at 250us. This experiment changes only the window
constant; the interpreter's exact-instruction behavior is unchanged.

A third prototype coalesces final private-arena physical retirements into
adjacent HVF unmaps, keeping the 250us window. A scoped per-thread transaction
retains slots and the VM until synchronous unmap completes; GPAs cannot be reused
early. It merges only exact adjacency in the same VM and never bridges a live
hole. External/native mappings, host-borrow and page-out transitions retain their
original behavior. All mapped bits in a run are cleared before releasing slots.
Tests cover a live hole, reuse, nesting and unwinding: 50 signed HVF tests plus
one application pressure test and workspace/all-targets Clippy pass.

This prototype improves single-CPU churn median work from 8.610ms to 7.925ms
(-8.0%), but six CPUs regress from 44.804ms to 46.957ms (+4.8%) and 12 CPUs are
flat (105.201ms to 104.888ms, -0.3%). Compute-peer total wall time is also flat
at 12 CPUs (281.367ms to 281.395ms). Separate diagnostic-only captures show why
the call reduction does not scale uniformly: single CPU unmap calls fall from
960 to 60, while the 12-CPU prototype queues 1331 private groups into 1058 unmap
batches, only about 1.26 groups per batch. Scheduling changes group counts, so
these are attribution captures rather than paired timing results. Diagnostic
per-operation clocks are excluded from all performance comparisons. This
prototype and its temporary common-runtime hook are removed too.

Across the three campaigns, all 270 fixed-work observations pass checksums.
The new C fixture also passes native AArch64 Linux oracles; the existing
`vm-checkpoint.c` supplies the compute-peer fixture. A reusable local harness
records shuffled order, hashes, every observation, guest/host durations and
checkpoint metrics, with an independent finite-work checksum oracle:

```sh
# Build these fixtures with an AArch64 Linux compiler, with assertions enabled.
cc -static -O2 -Wall -Wextra -Werror -pthread tests/fixtures/memory-paths.c -o /path/to/memory-paths
cc -static -O2 -Wall -Wextra -Werror -pthread tests/fixtures/vm-checkpoint.c -o /path/to/vm-checkpoint
# Run the following on the Mac with already signed, normal release executables.
python3 tests/integration/mac-memory-paths.py \
  --fixture /path/to/memory-paths --mapping-pause /path/to/vm-checkpoint \
  --variant baseline=/path/to/baseline-runner \
  --variant candidate=/path/to/candidate-runner \
  --cpus 1 6 12 --modes reuse churn peers --repetitions 5 \
  --output target/memory-path-comparison
```

The optional `--power-probe` executable emits `thermal_state` and `low_power`
JSON before and after the campaign. Keep other builds, tests, VMs and samplers
stopped. Guest `work_ns` begins before barrier release; `total_ns` includes
preparation and thread creation/join, but excludes final shared-array munmap.
Host duration includes complete process startup and teardown. The harness makes
no timing assertions and does not upload results. It is an opt-in experiment,
not an addition to routine CI. Existing production/shared-runtime/iOS checks are
reused because those sources and the installed executable are unchanged.

## Native Firefox graphics (2026-10-29)

The opt-in native Vulkan path now completes real Firefox hardware WebRender and
WebGL2 presentation through Zink, source-built MoltenVK and Metal. The common
resident-file snapshot and GPU checkpoint fixes are shared with iPad; Mac SDK
linking and on-demand presentation remain product-specific. See the
[acceptance and reproducible launcher](../validation/mac-firefox-gpu.md) for
exact scope, pixel checks and remaining video/driver limitations.


## Stable 1.4.0 Geekbench 6 retest (2026-11-01)

Installed stable 1.4.0 at product `dcba0098fce638ea2e646bd57ab03ff83ce1344d`,
paired runtime `1309faf688cd20780f4fa36d93e4f9d9ace8385f`, was retested with
original Geekbench 6.7.1 Linux AArch64 Preview and native macOS AArch64 builds.
Executables and workload archives match the previously verified originals.
The existing isolated Linux/glibc fixture is reused; this does not measure a
new Debian tool preset. Both platforms use 12 CPUs and report 24 GiB; Trames
uses host-managed `auto`, no windows/PTY, and the same enabled aggregate metrics
as the historical controls. No profiler, HVF diagnostics, syscall trace, build,
CI VM or browser capture ran during measurement. Normal desktop apps remained.
Trames ran first, followed by a 30-second pause and native. Both runs completed
and uploaded; thermal state was 0 and low-power mode off before and after each.

| Platform | Single-core | Multi-core |
| --- | ---: | ---: |
| Retained October 29 Trames controls, mean of two | 4,146.5 | 13,099.5 |
| [Trames 1.4.0](https://browser.geekbench.com/v6/cpu/19419637) | 4,310 | 13,828 |
| [Fresh native macOS](https://browser.geekbench.com/v6/cpu/19419653) | 4,427 | 20,236 |

Current Trames/native score ratios are **97.4% single-core and 68.3%
multi-core**. Trames raw scores rose 3.9%/5.6% against the retained controls.
The fresh native control also rose 3.3%/8.7% against the older native reference;
do not attribute the Trames raw-score increase solely to runtime changes.
This is one fresh sample per platform, not a confidence interval or isolated
HVF cost. Platform binaries, libraries, ISA dispatch and scheduling differ.

Each score pair below is single-core / multi-core. Previous Trames values
are the mean of the two retained controls, not the rejected prototypes.

| Workload | Previous Trames | Trames 1.4.0 | Native | Trames/native multi |
| --- | ---: | ---: | ---: | ---: |
| File Compression | 3,319 / 9,173 | 3,511 / 9,400 | 3,710 / 14,294 | 65.8% |
| Navigation | 3,270 / 14,272 | 3,480 / 15,768 | 3,805 / 20,531 | 76.8% |
| HTML5 Browser | 4,458 / 11,072.5 | 4,661 / 11,029 | 4,682 / 23,302 | 47.3% |
| PDF Renderer | 4,228 / 11,707 | 4,248 / 12,998 | 4,048 / 21,043 | 61.8% |
| Photo Library | 3,794 / 10,163.5 | 3,918 / 10,168 | 3,858 / 23,917 | 42.5% |
| Clang | 4,960 / 24,765 | 5,242 / 26,926 | 4,956 / 33,441 | 80.5% |
| Text Processing | 3,714.5 / 3,693 | 3,917 / 3,817 | 4,238 / 5,670 | 67.3% |
| Asset Compression | 3,615.5 / 18,005.5 | 3,732 / 17,758 | 3,797 / 26,775 | 66.3% |
| Object Detection | 6,525 / 13,084.5 | 6,688 / 12,957 | 6,390 / 18,361 | 70.6% |
| Background Blur | 4,297 / 13,144.5 | 4,396 / 14,077 | 4,488 / 20,940 | 67.2% |
| Horizon Detection | 3,974 / 20,749.5 | 4,123 / 22,472 | 4,759 / 27,735 | 81.0% |
| Object Remover | 5,057.5 / 15,450 | 5,407 / 16,763 | 5,596 / 22,540 | 74.4% |
| HDR | 4,329 / 18,707.5 | 4,466 / 19,474 | 4,990 / 23,133 | 84.2% |
| Photo Filter | 4,591 / 12,140.5 | 4,658 / 14,659 | 4,965 / 17,405 | 84.2% |
| Ray Tracer | 3,498 / 22,147 | 3,745 / 22,932 | 3,864 / 27,614 | 83.0% |
| Structure from Motion | 3,898.5 / 15,741.5 | 3,912 / 17,850 | 3,978 / 21,829 | 81.8% |

Photo Library and the headless HTML5 Browser workload retain the largest
multi-core gaps. These scores identify comparison targets, not their causal
bottlenecks, and are not Firefox GPU/presentation measurements. Full provenance,
raw scores, power state, durations and metrics are in the
[validation record](../validation/mac-geekbench.json). Production sources are
unchanged; the existing 1.4.0 local CI/runtime/iPad checks are reused.


### Unix IPC subsystem batch — 2026-11-03

Runtime `6ab059a7f0b57be7cab7e867c315ed8ed06ecd05` moves owning Unix SCM_RIGHTS graph admission,
cycle reclamation and shared native Wayland/XWM/clipboard adapters off ordinary
coordinator paths, shared with iPad. Guest notifications advance their connection
on the calling worker; native socket/pipe readiness uses the existing idle
executor, with no new service thread or cohort-control transport pump. Registry
locks cover membership only; independent connections retain independent progress.
True owner-only task/mount/proc projections remain their own resource boundary.

Four-process owning-rights loops improve **180.61 → 12.67 ms** (stream),
**185.59 → 16.03 ms** (raw discard/final EOF) and **221.71 → 49.30 ms**
(addressed datagram), with 1,000 checked iterations per process. Longer nine-sample
ordinary controls remain close; this is not a Firefox/GUI speedup measurement.
Three native Wayland cohort regressions complete with one cohort/zero services.
Original windows, SHM, Chinese editor input and native Vulkan rendering pass;
retained SHM/GPU frames survive session shutdown and release cleanly. The complete
native-linked development runner is installed separately from the stable product.
Mac/iPad builds, local Apple/Linux affected suites, strict checks and all thirteen
production FFI environment checks pass. Physical iPad validation remains skipped.

See the [full English audit and native comparisons](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/coordinator-removal.md#thirtieth-migration-implemented--unix-ipc-subsystem--2026-11-03)
and [source-bound validation record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-unix-ipc-20261103.json).


## Native device control subsystem — 2026-11-03

Shared runtime `ee0102c867930bb6b47cd6bc101ab471eed36a46` moves audio OPEN, video surface creation, live WSI
FD authority and explicit GPU device/instance destruction to execution workers.
Callbacks retain original event counters and the joined executor wake token,
including producers admitted after a task parks and after FD close/reuse. GPU
compilation/submission waits park existing jobs; final native driver destruction
retains its original algorithm. Frozen FD grants and their invalidation scans are
removed. No new coordinator/service thread or production unsafe is introduced.

Concentrated Apple/Linux suites, strict checks, native-linked Mac/iPad builds,
13 production FFI checks, five real Wayland stages, virtual duplex audio and
VideoToolbox/FFmpeg cross-process GPU import pass. The scoped video/GPU control
loops improve 184.09x/3.32x; 20 GPU cycles take 13.41 ms versus the same native
MoltenVK reference's 12.51 ms. Ordinary eventfd/pipe controls remain close. These
are API control benchmarks, not Firefox or physical-iPad FPS claims. Physical
iPad validation is skipped per owner. Stable/development installation stays separate.

Task/context cancellation, native guest mapping import, io_uring, job/task/proc/
mount control and structural filesystem/VM/task transitions remain boundaries.
See the [full audit](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/coordinator-removal.md#thirty-first-migration-implemented--native-device-controls--2026-11-03)
and [source-bound validation](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-native-devices-20261103.json).


## io_uring subsystem on source workers — 2026-11-03

Shared runtime `f9b6aec3a9eff34b7f733c2f1058684d49fbe80f` gives io_uring setup, registration, submission,
completion and background progress one canonical original ring state and mapped
byte domain. Source tasks retain their own execution context while their guest
CPU sleeps or waits elsewhere. Native fixed buffers retain the same SharedPage
and hardware borrow gate; remap cannot retarget accepted pins. Existing disk
workers handle plain-file fsync and notify the source without a coordinator RPC.
No new resource/service thread or polling bus is introduced. The implementation
is shared with the iPad interpreter, and HVF remains only the CPU/MMU backend.

All 66 real HVF io_uring cases pass at 1/2/4 CPUs, including IPv4/IPv6, multishot,
backpressure, fixed/provided buffers, cancellation, fork/exec and descriptor reuse.
Concentrated Apple/Linux ABI, memory and scheduler checks, strict checks, production
FFI environment checks and the native-linked iPad device target pass. Physical
iPad validation is skipped per owner. The four-worker fixed-registration workflow
improves 260.47 → 3.45 ms (75.56x); statx improves 102.92 → 30.94 ms (3.33x).
Single-core loop overhead regresses about 8–18%, with close process-wall medians;
filesystem metadata contention remains at four workers. The identical ARM64 ELF
on native Linux is a separate contextual reference, not a native macOS API score.

Owner-only task/mount/proc projections, buffer pin/VM topology mutation, job/task
controls and structural filesystem backing updates remain explicit checkpoints.
See the [full audit](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/coordinator-removal.md#thirty-second-migration-implemented--io_uring-subsystem--2026-11-03)
and [source-bound validation](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-uring-subsystem-20261103.json).

## Regular-file subsystem batch (2026-11-03)

Runtime `7739836e2302f8fb43f0a7b437f8038ae854ddd6` shares regular-file read/write,
append/growth, EOF changes, sync, O_TRUNC and file-to-file copy with the iPad
interpreter and io_uring. Short calls use the accepted original descriptor directly;
blocking work uses the existing disk pool and direct source readiness. Session
file RPCs, ring-only disk execution and per-cohort file metadata copyback are
removed. Actual mapped-layout changes quiesce only affected mm views.

Five-sample four-worker medians improve memory-backed independent-file growth
18.53 → 2.06 ms (9.00x), and private-disk growth 31.50 → 13.38 ms (2.35x), with
zero coordinator services. Contended shared-OFD private-disk append remains about
16% slower, reflecting actual resource/native-wait contention. Contextual native
macOS and exact-ELF Linux references are recorded without claiming equal backing
semantics or generally native speed. All affected file, asynchronous I/O, interpreter
and iPad link checks pass; physical iPad validation remains skipped per owner.

Directory/namespace mutation, owner-only task/proc/mount controls, cross-resource
file/pipe/socket transfers and structural VM/lifecycle remain separate boundaries.
See the [subsystem audit](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/coordinator-removal.md#thirty-third-migration-implemented--regular-file-subsystem--2026-11-03)
and [source-bound validation](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-file-subsystem-20261103.json).


## Namespace subsystem and coordination inventory (2026-11-03)

Runtime `af0aa49e60127236b2bf7858f95757306238ddee` shares ordinary directory,
metadata and canonical fs_struct operations with the iPad interpreter. Short work
uses the caller's current island; native work reuses the bounded disk pool and
original accepted resources. Excluded owner controls preserve a before-effects
handoff instead of copying guest operands or looking up reused FDs again.

Five-sample four-worker memory namespace lifecycle medians improve 1,195.47 →
127.74 ms (9.36x); cohort rounds fall 31,958 → 19. Service events are zero on both
versions, so service counts alone would miss the removed joined-owner cost.
Private metadata still retains per-operation durable manifest checkpoints:
single-worker latency grows 1,224.32 → 1,408.31 ms (15.0%), while two/four-worker
results have no material speedup claim. Native filesystem references differ in
backing and persistence guarantees. No whole-Firefox performance claim follows.

Task/signal/security and mount/proc controls, mixed owner resources and waits,
structural mm/backing publication, exceptional native mapping and embedding
lifecycle remain explicit boundaries. Some compatible FD/network calls still
use in-cohort owner service without pausing peers. Native libc fallback and HVF
coarse mapping comparison are different from owner checkpoints; AppKit affinity
is separate from kernel syscall coordination.

The [shared architecture-wide inventory](../../runtime/docs/architecture/coordination-boundaries.md)
is the current source audit. See the [batch validation](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-namespace-subsystem-20261103.json)
for concentrated Apple/Linux, actual HVF, interpreter and iPad native-link evidence.
Physical iPad testing remains skipped per owner. This closes the current round;
no further coordinator migration is started.

## Dev 1.5.3 Geekbench 6 retest (2026-11-06)

The installed Mac Dev runner uses product code `7816c12` and runtime
`a116d834318d59ea51f3de119fc7e20ed44c58d8` (scoped exec/process lifetime).
Its SHA-256 is `169c2dda603cf5cdc0b855358ff20d2e8de26a549ff617131a71ed29de61fffa`.
Later documentation-only runtime pins do not change the measured executable.
The unchanged original Geekbench 6.7.1 Linux AArch64 Preview and verified glibc
fixture reuse the isolated Alpine benchmark store; this is not the Debian preset.
The original macOS 6.7.1 binary ran afterward as a fresh native control.

Each platform ran once, sequentially, with 12 CPUs and 24 GiB reported memory;
Dev uses host-managed `auto`, no PTY/windows and aggregate metrics. No profiler,
HVF diagnostics, syscall trace, build, CI or agent window capture ran concurrently.
Existing stable sessions and normal desktop applications remained active: the
pre-Dev snapshot showed approximately 0.71 host CPU core used by three existing
Trames sessions. This snapshot is not whole-run load accounting. AC power,
nominal thermal state and low-power mode off were confirmed before/after both
runs; idle sleep was prevented. The interval between measured runs was
82.7 seconds. Both completed all 32 workloads
and uploaded, in 255.21 s (Dev) / 240.17 s (native).

| Platform | Single-core | Multi-core |
| --- | ---: | ---: |
| [Previous Trames 1.4.0, November 1](https://browser.geekbench.com/v6/cpu/19419637) | 4,310 | 13,828 |
| [Current Dev 1.5.3](https://browser.geekbench.com/v6/cpu/19437004) | 3,858 | 11,941 |
| [Fresh native macOS](https://browser.geekbench.com/v6/cpu/19437034) | 4,033 | 17,660 |

Dev is **95.7% single-core / 67.6% multi-core** of the fresh
native control. Against November 1, Dev changed -10.5% /
-13.6%; native changed -8.9% / -12.7%.
The historical comparison includes different background load and all intervening
changes. One sample per platform cannot isolate coordinator work or establish a
repeatable regression. Different OS binaries, libraries, ISA dispatch and scheduling
remain part of the platform comparison.

Selected multi-core components (the complete 16-workload single/multi table is
in [the JSON evidence](../validation/mac-geekbench.json)):

| Workload | Previous Trames | Current Dev | Fresh native | Dev change |
| --- | ---: | ---: | ---: | ---: |
| PDF Renderer | 12,998 | 7,484 | 17,435 | -42.4% |
| Background Blur | 14,077 | 10,974 | 17,427 | -22.0% |
| Clang | 26,926 | 21,523 | 28,024 | -20.1% |
| HTML5 Browser | 11,029 | 9,917 | 19,652 | -10.1% |
| File Compression | 9,400 | 10,177 | 12,616 | +8.3% |
| Photo Library | 10,168 | 11,715 | 21,202 | +15.2% |

PDF Renderer multi deserves a focused follow-up; scores alone do not establish
why it fell. The aggregate Dev session records 2,309,368 syscalls,
2,144,641 worker syscalls, 37,971 cohort rounds and
67,202 VM checkpoint groups (historical count:
7,154). These are whole-session counters,
not per-component profiles or additive wall-time categories. This run adds no
production changes; existing passing CI is reused and the stable application and
its sessions remain untouched. This is a Mac CPU benchmark, not iPad or Firefox
rendering acceptance.


## Original scheduling and demand-started CPU slots (2026-11-06)

Paired runtime `e6d700c3147a4fb5a2a772dfcbb28eb636ac53fc` moves the implemented Linux scheduling subsystem
to original task resources, shared with iPad. Source calls cover affinity,
nice/priority and scheduler metadata, including other tasks, groups/UIDs,
permissions, original migration completion and one seccomp decision per entry.
Normal bodies do not request an owner service or end the whole cohort. Tracing,
real policy lifecycle effects and previously unsupported features keep their
existing boundary/results.

Live affinity can now start a required cold slot directly through the original
shared CPU pool. The queue retains the original task/View, starts only needed
executors, and includes them in normal cancel/return/teardown accounting. No
resource service thread, polling bus or eager all-CPU startup is introduced.
Actual group/credential changes publish at their write boundary; normal queries
do not republish every owner projection.

| Workload, 12 CPUs | Previous loop | Current loop | Speedup | Native ARM Linux reference |
| --- | ---: | ---: | ---: | ---: |
| Self queries | 194.14 ms | 2.12 ms | 91.6x | 0.152 ms |
| Self updates | 189.99 ms | 2.14 ms | 88.8x | 0.375 ms |
| Cross-thread queries | 195.21 ms | 2.37 ms | 82.3x | 0.165 ms |

Each row is four threads × 256 loops × four calls, longest-worker median of five
counterbalanced unprofiled pairs, with two warmups per binary/configuration.
The update row reapplies the full mask, SCHED_OTHER/zero parameter and nice 7;
it is not continuous migration timing. Actual migration/cold slots have separate
compatibility acceptance.
Including complete process startup/teardown, the corresponding twelve-CPU
medians are 201.41 → 13.62 ms, 197.63 → 14.45 ms and 203.73 → 13.35 ms.
Native Linux is an exact-ELF contextual reference in the six-vCPU local CI
environment; macOS has no equivalent Linux affinity syscall ABI. Existing desktop
and stable workloads remain active. No builds, CI instance, trace, sampler or
window capture run during the Mac timings. Thermal state stays nominal.

| Workload, 12 CPUs | Previous cohort rounds | Current cohort rounds | Scheduling calls returned to owner |
| --- | ---: | ---: | ---: |
| Self queries | 3,936 | 1 | 4,096 → 0 |
| Self updates | 3,816 | 1 | 4,096 → 0 |
| Cross-thread queries | 3,791 | 1 | 4,096 → 0 |

Service requests are zero on both revisions; all 4,096 scheduling owner returns
disappear. The remaining whole-session return is the final control ioctl. These
measurements are scheduling controls, not a Firefox or Geekbench speedup. The
earlier Geekbench result above still describes runtime a116d83.

Apple/native ARM Linux validation passes 324 process and 65 kernel cases, strict
checks, both iPad compile targets, 56 actual HVF/interpreter fixture cases,
47 signed HVF cases, one pressure case, normal Mac product tests, 30 Go-GC rounds,
eight isolated Docker rounds and native GPU/video cleanup. Unchanged memory/VFS
and guest/native-driver artifacts are reused; physical iPad testing is skipped.
The exact measured static-MoltenVK Dev package is installed; stable/business
sessions are unchanged. See [the shared inventory](../../runtime/docs/architecture/coordination-boundaries.md)
and [complete stage 52 source, measurements and limits](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-scheduling-20261106.json).


## Dev 1.5.4 Geekbench retest and stable refresh (2026-11-13)

The installed Mac Dev runner uses runtime `0fd9a28` (product code `edcce3c`).
The stable Debian, Docker and default environments were stopped first. The local
stable application was then rebuilt/installed as 1.5.4 with the same runtime,
its signature verified, and all three environments left stopped. Existing
login-autostart preferences and persistent environment data are preserved.
No environment was started to verify the stable upgrade.

The unchanged original Geekbench 6.7.1 Linux AArch64 Preview and retained
Alpine/glibc benchmark fixture ran twice through installed Dev, with 12 CPUs,
host-managed auto memory (24 GiB reported), no PTY/windows and aggregate metrics.
No tracing, sampling, build, CI or other benchmark ran concurrently. Both
attempts exited 1 near the multi-core Asset Compression/Object Detection
transition with `all virtual processes are blocked without an event source`.
Both completed all single-core workload announcements, but neither completed the
benchmark or produced a valid aggregate single/multi score or public result.
The first failure and identical-binary retry are both retained; no old score is
substituted. Whether this is a false idle decision or a lost guest wake remains
unresolved. Production sources and runtime pins were not modified for the run.

The original macOS 6.7.1 control ran sequentially between the Dev attempts and
completed in 235.25 seconds: **4,459 single / 20,451 multi**.
[Public native result](https://browser.geekbench.com/v6/cpu/19465807).
Thermal state was nominal and low-power mode off before/after all attempts;
idle sleep was prevented. The public result was read after measurements ended.
There is no current Dev/native performance ratio because Dev did not finish.
[Filtered evidence](../validation/mac-geekbench.json) retains all 16 native
components, both failed attempts, execution metrics and stable upgrade hashes.
Raw logs remain in the ignored `target/geekbench-dev-20261113` directory.


## Terminal idle fix and completed Geekbench run (2026-11-13)

Runtime `0416b41` replaces production's loose queue-idle recheck with one
terminal-quiescence transaction. A native-service probe can release the queue
mutex, but its observation is invalidated by runnable publication or a newer
probe. Final validation repeats source, timer, GPU, external wait, retirement
and admission conditions; it shares the queue lock with admission closure.
A valid native service parks without a busy retry loop, while changed work or
deadlines are processed again. The reason is published through the existing
one-way control word. Ordinary dispatch gains no counter, atomic, memory barrier
or lock; the probe token increments only on suspected terminal idle. No
coordinator, service thread or polling bus is introduced.

All 487 shared session/process regressions pass, with three unchanged manual
profiles ignored. New tests exercise probe-gap changes, ready/run/repark,
superseded probes, prior normal exit and actual guest futex deadlock. Strict
core/Mac checks, 22 Mac product tests, iPad ARM64 FFI and execution/completion
boundaries pass; unrelated instruction/GPU/network checks are reused.

The installed fixed Dev runner completes and uploads all 32 unchanged original
Geekbench 6.7.1 workloads in 240.34 seconds, using 12 CPUs and host-managed memory
(24 GiB reported). No build, CI, profiler or window capture runs concurrently;
thermal state is nominal and low-power mode off before/after. Stable environments
are stopped. The isolated fixture's hosts entry is refreshed from native DNS,
and the original glibc getent confirms resolution before the run.

| Geekbench 6.7.1 | Single | Multi |
| --- | ---: | ---: |
| [Fixed Dev](https://browser.geekbench.com/v6/cpu/19465924) | 4,335 | 16,156 |
| [Same-session native control](https://browser.geekbench.com/v6/cpu/19465807) | 4,459 | 20,451 |
| Dev / native | 97.2% | 79.0% |

HTML5 Browser multi scores 23,008 versus 23,880 (96.3%); PDF Renderer multi
remains 8,933 versus 21,182 (42.2%). These Linux Preview/macOS results include
binary, library and ISA-dispatch differences and are not pure HVF overhead.
One successful fixed Dev sample is measured. The exact interleaving in the two
original failing runs was not captured: unfixed terminal-error-only diagnostic
builds also completed computation but failed DNS upload. Deterministic tests
establish defects in the old idle protocol; the successful benchmark is not
proof that every historical failure had the same sole cause. All attempts and
16 component comparisons remain in the [JSON evidence](../validation/mac-geekbench.json).

Both Mac Dev and the local stable app are signed and updated to this runtime.
The stable Debian, Docker and default environments remain stopped; they were not
started to verify installation. Installed iPad apps are unchanged.


## Geekbench Vulkan GPU completion (2026-11-13)

The original Geekbench 6.7.1 Linux/AArch64 Vulkan benchmark now completes on
installed Mac Dev 1.5.4, runtime `b73f42b`. The isolated Alpine/glibc fixture uses
an unmodified Ubuntu Vulkan loader, the current wire-20 ICD built against dynamic
glibc, and `LC_ALL=C LANG=C`. The device is Apple M6 through the embedded original
MoltenVK/Metal path. Geekbench binaries and workloads are unchanged.

The first attempt failed during Background Blur: a diagnostic captured a single
`VkSubmitInfo` with 165 command buffers, rejected by the guest's fixed 64-entry
packing arrays. The host also imposed 64-record/per-array/aggregate checks.
Legacy queue submission now shares Sync2's existing stack/spill arena and owned
size-checked host readers. Small calls stay on the stack; large arrays follow
the application's counts. Original submit boundaries, native ordering and one
native queue submission are preserved. No coordinator/service thread, global
lock, polling path or workload/submission splitting is added.

All 78 native Vulkan regressions pass, none ignored. The expanded identical
native/guest output oracle checks 165 command buffers, 165 submission boundaries,
and 65 binary/nonzero timeline semaphore values. It passes directly on native
MoltenVK and under glibc/musl guests at 1 and 12 CPUs. Strict Vulkan/Mac lints,
22 Mac tests, ARM64 iPad FFI and formatting pass. The canonical musl guest ICD
is rebuilt and source-manifest-verified for future packaging. Unaffected CPU,
Linux, networking and execution-lifecycle checks are reused.

| Geekbench 6.7.1 GPU | API | Score | Full CLI wall time |
| --- | --- | ---: | ---: |
| [Fixed Dev](https://browser.geekbench.com/v6/compute/6971838) | Vulkan | 5,876 | 355.30 s |
| [Native host](https://browser.geekbench.com/v6/compute/6971818) | Metal | 94,256 | 65.06 s |

| Workload | Dev Vulkan | Native Metal | Vulkan / Metal |
| --- | ---: | ---: | ---: |
| Background Blur | 2,478 | 45,644 | 5.4% |
| Face Detection | 2,208 | 71,119 | 3.1% |
| Horizon Detection | 1,213 | 83,974 | 1.4% |
| Edge Detection | 1,244 | 86,193 | 1.4% |
| Gaussian Blur | 1,779 | 138,953 | 1.3% |
| Feature Matching | 3,842 | 31,917 | 12.0% |
| Stereo Matching | 125,954 | 291,884 | 43.2% |
| Particle Physics | 200,048 | 204,825 | 97.7% |

This is one sample per API, including Linux Preview/macOS binary, library and
Vulkan/MoltenVK versus Metal differences; it is not pure runtime overhead or a
same-API native Vulkan comparison. The functional blocker is fixed, but image
workload performance remains poor. The full Dev run records 187.7 GB of coherent
upload scanning, 4.05 GB of download scanning, 1,810 pipeline compiles and 195.4 s
of aggregate GPU waits. These counters do not provide exclusive time attribution.
No profiler or window capture runs during scoring; performance analysis must
separate these costs before assigning a cause.

Both runs exit zero and upload valid public results. Thermal state is nominal
and low-power mode off before/after. Compiler VM, stable environments, CI and
other benchmarks are stopped during measurement. Installed Dev is updated;
stable remains on `0416b41` with Debian, Docker and default environments stopped.
Installed iPad apps are unchanged. The [filtered evidence](../validation/mac-geekbench-vulkan-20261113.json)
retains the original rejection, error-only diagnostic, expanded correctness
oracles, exact source/binary hashes, all eight scores and execution counters.
Raw artifacts stay ignored under `target/geekbench-gpu-20261113`; upload claim
links are not published.


## GPU mapping cliff and stable recovery (2026-11-13)

[Same-API measurements and stack attribution](../validation/mac-gpu-overhead-20261113.md)
identify shadow coherent-memory synchronization as the dominant sampled worker
path in the slow Geekbench image phases (76.3–93.2% of that worker's sampled
stacks, not whole-run exclusive time). A legal 4 KiB allocation tail changes a
100-submit Dev Vulkan control from 27.96 ms to 2,410.37 ms, while GPU timestamp
time remains around 1 ms. The same native Vulkan control has no such cliff.
Async poll-side readback is missing from aggregate transfer counters, and pending
call latency includes CPU readback; earlier `gpu_wait_us` is not pure GPU time.
Generic native backing ownership/alignment is the first proposed optimization;
no GPU memory-policy change or new scored Geekbench result is claimed here.

Restoring stable also exposes an independent current-main runc/zombie-overlay
startup regression. The deterministic metadata-only overlay occupancy bug is
fixed in `dfe6ff9`, but does not resolve the whole startup failure. Stable service
is recovered using the preserved signed formal 1.5.4 (`eb047dc`), with both original
containers running and the prior web endpoint returning HTTP 200. Dev retains
`dfe6ff9`. Data, configuration and autostart choices are preserved. Current-main
container startup remains an open release blocker; see the linked report.


## Native coherent backing without shadow scans (2026-11-13)

Runtime `619d14e` rounds only generic native coherent backing to complete owned
host mapping granules, retaining exact guest bounds, native memory/image aliases,
last-view ownership and actual-byte budgeting. Adoptable original pages directly
back guest maps, including legal allocation tails. Dedicated/external contracts
and the non-adoptable fallback remain. Async completion transfer accounting now
includes its readback delta. No coordinator/service thread or instruction/write
instrumentation is introduced.

The same-session installed Dev Vulkan Geekbench score rises from 5,841 to 62,420
(10.69x); whole invocation falls from 355.52 to 65.09 s. The complete fixed run
has zero coherent scans/shadow transfers. A fresh native Metal reference scores
94,426; that API/binary difference is not pure runtime overhead. The identical
Vulkan control's legal 4 KiB-tail case falls from 2,410.37 to 16.93 ms versus
14.91 ms native, with actual GPU output/timestamp checks. See the
[complete follow-up](../validation/mac-gpu-overhead-20261113.md#implemented-original-shared-coherent-backing)
and [exact evidence](../validation/mac-gpu-shared-backing-20261113.json) for all
workloads, ownership/partial-map/iPad checks and measurement limits.

Only Dev is installed from the new pin. Stable remains the recovered formal
1.5.4 `eb047dc`, with both deployments running; the independent current-main
runc startup blocker remains unresolved. Installed iPad apps are unchanged.


## Original FIFO lifetime and allocation publication (2026-11-13)

Runtime `86dfa2c` completes blocking FIFO rendezvous on the original endpoint for
ordinary/file-work/io_uring opens; pending opens release executor slots and own
exact cancellation cleanup. Exited fs_struct membership releases cwd/root use;
pivot_root and later query writes cannot recreate leases for retired tasks.
This fixes runc exec.fifo EBADF and Docker overlay remount failure without a
coordinator or weakened overlay exclusivity.

Canonical non-executable imported data publishes its existing allocation epoch
once per joined hardware boundary. Writable mapping membership is retired on
unmap/protection changes; executable and file-backed hooks stay page-based.
The controlled 256 MiB mapping/layout probe falls from 260.55 to 38.18 ms vs
32.14 ms same-API native Vulkan. Final Geekbench Vulkan is 63,175; the score does
not establish a broad steady-state gain. See the
[Docker/GPU validation record](../validation/mac-docker-gpu-20261113.md) for
mechanisms, original output checks, limits and release acceptance.
