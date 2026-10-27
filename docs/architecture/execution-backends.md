# Hardware execution and pageable memory

`trames-mac` is a terminal product over the same `trames-session` Linux userspace
kernel as `trames-ipad`. ARM64 instructions execute in Hypervisor.framework; a
small EL1 adapter returns SVC, faults and timer exits to the host kernel. No full
Linux kernel, boot RAM image, interpreter, or product-specific syscall kernel is
started. [Issue 44](https://github.com/kb5000/trames-ipad/issues/44) tracks remaining
product integration gates.

## VM and CPU ownership

There is one HVF VM per host process. Each existing execution worker lazily
creates its own thread-affine vCPU and private stage-one tables. Movable Linux
task state includes all general/SIMD registers, NZCV, stack and TLS; it never
moves a vCPU between threads. A distinct ASID and non-global PTEs isolate each
worker's virtual translations. Table updates invalidate that ASID, rather than
all peers' translations. ASID encoding and TLBI operands follow the ARM64 layout
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
binaries, then execute the seven hardware/pressure checks.

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

This is a design review, not an implemented memory-contract change. The working
recommendation is to keep the shared Linux userspace runtime and redesign its
HVF memory integration for the Mac product. Native ARM64 execution with syscall
rewriting remains a bounded alternative experiment. A full Linux VM is a valid
separate product strategy when broad kernel compatibility outweighs sharing the
iPad kernel and directly controlling guest resources.

The decision includes memory, process isolation, storage, networking, display,
and maintenance. An empty syscall benchmark alone cannot select the backend.
The existing [CoreMark comparison](../validation/mac-compat-coremark.md) establishes
near-native compute for that workload; [Firefox CPU sampling](../validation/mac-windows.md#cpu-baseline)
finds most CPU in permission changes and mapping maintenance. Neither measures
the intrinsic cost of an otherwise unchanged HVF syscall exit in isolation.

| Candidate | Whole-system implication |
| --- | --- |
| Redesigned HVF with the shared runtime | Hardware guest address spaces and ordinary ARM execution, with current shared VFS/socket objects and iPad kernel reuse; host crossings remain. |
| Native ARM64 with syscall rewriting | Removes virtualization crossings, but overlapping guest address spaces require separate host processes or address translation; resource and code-lifetime models must change. |
| Same-ISA dynamic translation | Can retain one host process and call the runtime directly; must account for address translation, code cache, indirect branches and JIT invalidation. |
| Full Linux VM | Linux owns process, filesystem and network semantics; requires new host integration and cooperative memory return, with different kernel implementations on Mac and iPad. |

### Memory and process ownership

The target is no hardware permission toggle for routine metadata retention or
ordinary bounded syscall transfers, and no full cached-PTE walk on an unchanged
address space. Stable physical mappings should last while resident; persistent
translation state should belong to a Linux address space, with actual mapping,
permission, COW and reclamation changes applied incrementally. A content/code
revision is distinct from translation identity. The current complete
`HardwarePageInfo` comparison can retire executable translations after a shared
code epoch changes; its contribution to observed churn still needs measurement.

Guest bytes and host Rust objects need distinct access contracts. Metadata-only
handle retention must not imply byte borrowing. Bounded copy-in/out or audited
raw ARM operations are candidates for concurrent guest bytes; existing APIs
that create guest Rust slices must continue excluding conflicting hardware
access until their contract is changed and validated. This review does not
authorize simply deleting those gates. Actual mapping lifetime, first-write COW,
file truncation and architectural synchronization remain necessary. Fork should
eventually detach private storage on the first write rather than eagerly during
View admission.

Native execution in a host process per guest address space obtains host VM
isolation, but current in-process Arc/Mutex resource objects cannot be shared
unchanged across those processes. Some sharing can move to inherited host file
descriptors; virtual namespace ownership, Linux-specific state and cross-process
coordination still need a design. Same-process native code instead needs a way
to represent overlapping Linux virtual addresses and preserve the guest/host
memory boundary. A syscall trampoline alone does not supply that mechanism.

Reclamation acceptance must distinguish logical allocation, live resident data,
host RSS/physical footprint, caches and backing-file disk usage. Reserving a
4 GiB guest budget is not the same as populating it. Unmapping or advising away
guest pages should release ownership; pressure reclamation of live dirty bytes
must preserve their data. The current backing file reuses freed offsets but
retains its disk high-water mark until teardown, and synchronous page-out can
create disk stalls. Memory savings must therefore be measured alongside backing
space and I/O. Allocator-retained guest heap is not automatically known free.

### Storage and networking

Keep one byte authority for file I/O and shared mmap, including truncate/EOF,
open-unlinked files, permissions, quotas and durability. Existing direct file
capabilities already use shared open-file objects and host handles; mapped
files intentionally leave stale direct-disk paths. CPU backend replacement does
not remove these coherence rules or current gather/scatter copies. Small I/O
should use bounded reusable buffers; large operations need an explicit retained
buffer/pinning/cancellation design before a zero-copy claim. Blocking disk work
must release execution slots through existing scheduling facilities.

FileProvider export and staged writeback remain an explicit synchronization
boundary. Exposing internal object files as an independently writable host tree
would bypass namespace/mmap authority. A full VM would need an export/writeback
agent or protocol for its internal Linux filesystem. A host directory exposed
through [virtiofs](https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdeviceconfiguration)
is a separate shared mount, not automatic host access to a guest block rootfs.

Ordinary root TCP/UDP already use nonblocking host BSD sockets. There is no
packet-stack/NAT hop to eliminate on that path. Preserve direct shared object
access and readiness-based parking; reserve packet processing for explicit
virtual network semantics. Linux socket options, AF_UNIX/descriptor passing,
epoll, namespaces and DNS behavior remain compatibility work whichever CPU
backend is used. A full VM delegates those semantics to Linux and uses a device
boundary for external traffic.

A full VM need not imply unavoidable double disk caching: Apple's
[disk image cache modes](https://developer.apple.com/documentation/virtualization/vzdiskimagecachingmode)
include uncached operation. Durability/synchronization must still be preserved.
Its [memory balloon](https://developer.apple.com/documentation/virtualization/vzvirtiotraditionalmemoryballoondevice)
requests guest cooperation; the SDK explicitly says the guest may not handle
the request. Configured RAM alone proves neither fixed host RSS nor immediate
reclamation. These require the same real footprint measurements as other routes.

### Selection gates

Compare unchanged-mapping HVF syscall exits against native trampolines, then
measure whole operations: multiple address spaces at identical VAs; shared
mmap/futex/FD/socket behavior; fork and first-write COW; JIT RW/RX aliases;
random 4 KiB and large sequential I/O; fsync/rename durability; mmap/truncate;
TCP and AF_UNIX throughput and tail latency; epoll wakeups; display/input and
FileProvider conflict handling. Record CPU and blocked time separately, as well
as peak/retained footprint, backing-space use, and reclamation/recovery latency.
Native promotion requires these gates, not just a faster empty syscall loop.

The current SDK has 4 KiB process-page and custom-x18 ABI interfaces, so old
blanket claims that native execution is impossible are inappropriate. Capability,
signing, signals and JIT integration remain unproven for this product. No new
backend or unsafe guest-memory implementation is selected by publishing this
review; raw experiments and traces stay in ignored local build output.
