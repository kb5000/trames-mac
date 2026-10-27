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
toplevel owns a native window; outlying subsurfaces and popups use bounded child
rectangles instead of a desktop-sized transparent input surface. The last
window releases cached Metal setup and removes the Dock presence without
stopping Linux or its terminal.

Metal reads retained file-backed SHM mappings directly. No Rust byte slice or
copy of the guest framebuffer is introduced by the presenter. A snapshot lease
owns the source until GPU reads finish. Positive frame acknowledgement also
waits for every drawable's actual presentation; skipped, cancelled, and occluded
frames receive no successful presentation acknowledgement. Invalidated run-loop
sources remain retained until pending GPU callbacks finish, so teardown cannot
call a freed UI context. Frame cancellation and multi-drawable completion have
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
