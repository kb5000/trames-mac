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
storage retires the leaf; content-only changes preserve it. Expired history or a
full snapshot replacement requires full reconciliation. Conservative dirty
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

## Physical paging

The default 4096MiB guest limit is a logical allocation budget, not preallocated
RAM. Only accessed pages acquire physical slots. Four unrelated 4KiB guest pages
can share one 16KiB host granule, and partially freed granules are reused before
allocating another. Mapping addresses need not be contiguous.

Canonical storage uses one unlinked, mode-0600, close-on-exec temporary file.
Disk space is reserved in checked 1MiB extents before writable pages are
published. Freed file offsets are reused; file size reflects the session's high
water mark and is released when its final mapping/owner ends. Host volume
protection applies; Trames does not add separate backing-file encryption.

Reclamation withdraws the stage-two mapping, preserves dirty data with
`MS_SYNC`, then invalidates host residency. The stable MAP_SHARED host address
remains valid. Hardware access remaps the granule; native access faults the same
file-backed bytes in through the OS. Final host borrows of evicted granules also
release their restored host residency. Failed backing I/O does not discard the
only live data copy, and initial backing allocation failures remain host errors.

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
device bytes, RSS/footprint, backing disk use and recovery latency. Current
backing storage retains its disk high-water mark until teardown, so RAM savings
must be measured alongside disk space and page-out stalls.

Both products must pass the affected common semantic tests. Common ABI/memory
changes also require an iPad target check and relevant interpreter/native peers;
HVF-specific mapping tests and AppKit tests stay on Mac. Reuse unchanged passing
scopes. This documentation change adds no execution backend, device capability
or unsafe memory implementation; raw experiments remain in ignored local output.
