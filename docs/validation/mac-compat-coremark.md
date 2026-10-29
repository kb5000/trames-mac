# Mac Linux compatibility, CoreMark and Geekbench

This checkpoint enables direct EL0 virtual-counter/frequency reads and converts
privileged system-register traps into recoverable guest SIGILL. Before the fix,
both fixture modes terminated the backend with an EC=0x18 fault. The same
static-musl ELF passes on native ARM64 Linux; after the fix it passes on HVF
with one and four execution slots. Timer-control and EL1 registers remain
privileged, while ordinary counter reads execute directly in hardware.

## Measurement method

The upstream benchmark is pinned to
[EEMBC CoreMark 1f483d5](https://github.com/eembc/coremark/tree/1f483d5b8316753a742cbf5590caf5bd0a4e4777).
The source hashes, including all five algorithm/framework C files and the POSIX
port, are checked before compilation. The port alone switches CLOCK_REALTIME to
CLOCK_MONOTONIC in both binaries. The algorithms are unchanged.

The same Apple Clang builds Mach-O/macOS and static ELF/Linux with `-O2 -fno-lto
-mcpu=generic -march=armv8-a -ffixed-x18 -fomit-frame-pointer`. Explicit generic
tuning avoids the different implicit Darwin and Linux CPU defaults. Reserving
x18 and selecting frame-pointer omission apply equally to both targets. Linux
uses musl and its AArch64 static compiler builtins; macOS uses the system C
library. Platform ABI, generated code, standard libraries and scheduling remain
part of this end-to-end comparison.

Each configuration uses 2000 heap bytes per context, performance seeds
`0 0 0x66` and validation seeds `0x3415 0x3415 0x66`. One or four pthread
contexts are built separately. A short calibration chooses one common iteration
count for both targets in that thread configuration; calibration is excluded
from scores. Three scored repeats alternate native/HVF order. Every scored run
must exceed ten seconds, pass upstream CRC validation, report the requested
iteration/thread counts and start/end at nominal thermal state with Low Power
Mode disabled. Caffeinate prevents idle sleep. macOS chooses CPU placement;
there is no core affinity. The project Linux CI VM was stopped and no concurrent
builds ran during the recorded benchmark.

The HVF program runs through the normal full Linux session with a verified
Alpine rootfs, guest process loader and shared syscall kernel, `--cpus 1` or
`--cpus 4`, a 4096MiB logical memory budget and no optional resident target.
The CoreMark timer excludes rootfs installation/process startup. Separate wall
and syscall/paging metrics are retained. These are development measurements.

## Reproduce

Use a native Linux AArch64 static-musl sysroot containing headers, startup
objects and libc.a; also transfer its AArch64 libgcc.a or compiler-rt builtins
archive. This only links the Linux C library support: all benchmark C files for
both targets are compiled by the same Mac Clang. LLVM rust-lld from the pinned
Rust toolchain supplies the cross linker.

```sh
bash scripts/build-mac.sh
# Download and unpack the fixed upstream archive. SHA-256:
# 4067e7f260218df13f2875d8f821a8ff32a84153c6e0759d7c89f96c0a8bf987
python3 tests/performance/coremark.py \
  --runner target/release/trames-mac-run \
  --rootfs /path/to/alpine-minirootfs-3.22.6-aarch64.tar.gz \
  --source /path/to/coremark-1f483d5b8316753a742cbf5590caf5bd0a4e4777 \
  --sysroot /path/to/aarch64-musl \
  --builtins /path/to/aarch64-musl/lib/libgcc.a \
  --linker /path/to/rust-lld \
  --output target/coremark-comparison

# Compile the fixture on native AArch64 Linux, or with the same Clang/sysroot.
musl-gcc -O2 -static -pthread tests/fixtures/cpu-access.c -o cpu-access
python3 tests/integration/mac-cpu-access.py \
  --runner target/release/trames-mac-run --fixture /path/to/cpu-access \
  --output target/cpu-access
```

## Recorded result — 2026-10-27

Code checkpoint: [`79a37b5`](https://github.com/kb5000/trames-mac/commit/79a37b50498193b92963bfec7883d06805e79d0f).
Runtime: `fa0310b84bc34788d2c01b15c6923f4acfd3301c`. Host: **Apple M6**, macOS 27.0.1;
compiler: **Apple clang version 21.0.0 (clang-2100.3.34.2)**. All 24 scored samples passed CRC, duration,
thread/iteration-count and start/end power/thermal checks. Scored durations were
**11.543–14.052 seconds**. Every four-vCPU sample reached `peak=4`.

| Contexts | Input | Native median iterations/s | HVF median iterations/s | HVF / native |
| ---: | --- | ---: | ---: | ---: |
| 1 | performance | 59,280 | 58,846 | 99.27% |
| 1 | validation | 59,248 | 59,095 | 99.74% |
| 4 | performance | 207,666 | 207,379 | 99.86% |
| 4 | validation | 206,026 | 205,744 | 99.86% |

Four-context performance scales **3.52x** over one context on HVF and **3.50x**
on native macOS. Aggregate four-context throughput is reported above; each
context has its own 2000-byte working data.

Single-context performance ranges were 56,485–59,949 native and 55,508–59,406 HVF;
their coefficients of variation were 2.56% and 2.97%. The median difference is
smaller than that spread. Four-context repeats were steadier (about 0.1% CV).
These samples establish near-native compute throughput for this workload;
they do not isolate a sub-percent virtualization cost or predict I/O-heavy
application performance.

Fresh local validation also passed: complete Mac format/clippy/unit/build
checks, six signed hardware/pressure tests, four register-access cases, six
Linux session/PTY/persistence cases, four native-Linux signal oracles and three
live SMP/paging cases. The common runtime production code is unchanged, so its
previous complete 81-stage native Linux validation was reused.

[Results, exact hashes and validation](mac-coremark/results.json) ·
[Original output for every scored/calibration run](mac-coremark/outputs.json) ·
[All compile/link commands](mac-coremark/build-commands.json)

## Geekbench 6.7.1 — 2026-10-28

Geekbench adds larger working sets, allocation, threading, libraries and mixed
application kernels to the small CoreMark compute test above. This is one
completed sample per platform, not a repeated statistical estimate or a Firefox
responsiveness measurement. Code checkpoint: `7a54477cde6c988cd264f3534a612f15a28354f6`;
shared runtime: `dc49cbfed76fa5876dcc1ac2eb0e49100b464c26`.

### Method and compatibility

Use the matching official **6.7.1** Mac AArch64 and Linux AArch64 Preview packages,
from the [legacy](https://www.geekbench.com/legacy/) and
[preview](https://www.geekbench.com/preview/) downloads. The Preview is a real
Linux ELF running in the normal Trames userspace Linux session through HVF;
there is no full Linux-kernel VM. It needs glibc. Keep the pinned Alpine rootfs
and import six unmodified libraries from the checksum-verified Ubuntu Base
24.04.5 ARM64 archive into `/lib`: `ld-linux-aarch64.so.1`, `libc.so.6`,
`libm.so.6`, `libdl.so.2`, `libpthread.so.0` and `libgcc_s.so.1`.
Import the original `geekbench6`, `geekbench_aarch64` and both `.plar` files into
`/opt/geekbench`; no benchmark instructions or data are patched.

The completed Trames sample uses **4 execution slots and a 4096MiB logical
budget**; native macOS uses all **12 host cores and 24GiB**. This is an absolute
four-slot product measurement against the full host baseline, not an equal-core
multi-core overhead comparison. The guest reports four cores. The product's
4096MiB default is unchanged and no optional resident target is set. Mac
hardware/OS and normal desktop background applications are shared; CPU
placement is left to macOS.

Stop the test Firefox before measurement and prevent idle sleep with
`caffeinate`. Run sequentially: the failed 12-slot/4096MiB attempt, native
baseline, an offline traced memory diagnosis, a diagnostic 12-slot/8192MiB
attempt, a 4-slot/4096MiB run whose upload failed after all workloads, and
finally the completed uploaded 4-slot/4096MiB sample. No builds, window capture,
sampling profiler, process-state sampler or syscall trace run during the
completed scored measurements. Commands omit `--metrics` and `--trace`.
Completed scored runs start and end at nominal thermal state with Low Power
Mode disabled. The owner authorized automatic upload of one completed result
per platform. Public score URLs are retained, while raw output and result-claim
credentials remain private under ignored `target/`.

```sh
trames-mac-run --storage /path/to/geekbench-store \
  --network outbound --cpus 4 --memory-mib 4096 --timeout 3600 \
  --no-pty --no-windows /bin/sh -c \
  'cd /opt/geekbench && LD_LIBRARY_PATH=/lib ./geekbench6 --cpu'
'/path/to/Geekbench 6.app/Contents/Resources/geekbench6' --cpu
```

### Result

| Platform | Cores / memory | Single-core | Multi-core | Trames / native |
| --- | --- | ---: | ---: | --- |
| [Trames Linux/HVF](https://browser.geekbench.com/v6/cpu/19400701) | 4 slots / 4GiB logical | 2,421 | 2,795 | 56.5% single / 15.0% multi |
| [Native macOS](https://browser.geekbench.com/v6/cpu/19400588) | 12 cores / 24GiB host | 4,287 | 18,616 | 100% |

The single-core aggregate reaches **56.5%** of this native sample. Single-core
Clang, HTML5 Browser, Asset Compression and Ray Tracer reach approximately
93–98%; Object Detection reaches 14.5% and Photo Filter 20.3%. This distribution
is consistent with multiple mechanisms, rather than one uniform instruction
execution penalty; capability/algorithm dispatch needs separate verification.

The Trames multi-core aggregate is only **1.154x** its own single-core score.
Some workloads get slower in the multi-core phase: PDF Renderer scores
1,009 versus 2,977 single-core, HTML5 Browser 2,930 versus 4,146, and Photo Filter
284 versus 964. Unequal host/guest core counts do not explain those regressions
within Trames itself. These are concrete targets for concurrency and large
working-set diagnosis; the scores alone do not identify the responsible lock,
fault or memory-management path. Prioritize that scaling problem before using
the small CoreMark result to characterize general application speed.

| Workload | Trames / native single-core | Trames / native multi-core |
| --- | ---: | ---: |
| File Compression | 46.2% | 13.2% |
| Navigation | 31.6% | 7.5% |
| HTML5 Browser | 92.8% | 13.8% |
| PDF Renderer | 75.0% | 5.2% |
| Photo Library | 58.5% | 10.6% |
| Clang | 95.9% | 33.2% |
| Text Processing | 57.8% | 32.6% |
| Asset Compression | 95.3% | 22.9% |
| Object Detection | 14.5% | 8.9% |
| Background Blur | 47.5% | 21.8% |
| Horizon Detection | 79.4% | 23.2% |
| Object Remover | 85.2% | 52.5% |
| HDR | 60.9% | 10.8% |
| Photo Filter | 20.3% | 2.0% |
| Ray Tracer | 97.5% | 33.2% |
| Structure from Motion | 68.0% | 31.1% |

The native result reports NEON, AES/SHA, FP16, dot-product, I8MM and SME/SME2
capabilities. The current guest advertises only FP+ASIMD (`HWCAP=3`,
`HWCAP2=0`); Geekbench reports `neon`. Capability reporting does not establish
which instructions a particular workload actually used. The core-count
difference further affects multi-core scores. The gap includes
instruction dispatch, Darwin/Linux builds and ABIs, glibc/system libraries,
logical memory limits, runtime memory/syscall work and scheduling. It is not a
measurement of isolated HVF overhead, nor proof that the same ratio applies to
Firefox. Do not combine Geekbench 6 scores with Geekbench 7 scores.

### 4096MiB allocation limit

The original 12-slot/4096MiB attempt completed all 16 single-core workloads, then
exited 255 during multi-core **Navigation** with `std::bad_alloc`. It produced
no score or upload. A separate offline, traced run reproduced this failure.
Successful `mmap`/`mprotect`/`munmap` ranges reconstruct approximately
4,006,141,952 mapped bytes just before a 335,560,704-byte RW `mmap` returns
`ENOMEM`. That sum exceeds the 4GiB per-space mapping limit in `Memory::map`.
The reconstruction excludes initial ELF pages and some PROT_NONE file mappings;
it is an estimate, not a unique physical-memory measurement. Sampled peak host
RSS in this diagnostic run was 3,456,208KiB; RSS may count aliases and sampling
can miss the instantaneous peak.

This exposes the existing distinction between the logical mapping quota and
first-write/resident accounting. A separate 8192MiB diagnostic attempt
gets past Navigation; it does not prove that a native 4GiB Linux
system would also run out of physical RAM. Do not silently redefine the shared
runtime's quota policy to improve a benchmark score. Diagnostic timings are
excluded from performance results.

### Upload compatibility

The first four-slot/4096MiB run completed all 32 workloads in 457.24 seconds,
then exited 255 when libcurl could not resolve `browser.geekbench.com`.
It retained no result file to upload later. This is a compute-complete run with
no score, not an uploaded benchmark result.

A short syscall trace of the original Ubuntu `getent` isolates a glibc DNS
compatibility gap: glibc sets `IP_RECVERR` (`SOL_IP=0`, option 11); the current
runtime returns `ENOPROTOOPT` (92), so glibc abandons DNS before sending a query.
The same guest's musl BusyBox `nslookup` resolves the name. For this benchmark
only, resolve the Geekbench hostname through the host system resolver and add
it to the **isolated benchmark guest's** `/etc/hosts`. Both glibc `getent hosts`
and `getent ahostsv4` then succeed; an HTTPS preflight reaches the server and
receives HTTP 403. Provision the public CA bundle from
[the curl CA extract](https://curl.se/docs/caextract.html), verify its published
SHA-256, and place it at `/etc/ssl/certs/ca-certificates.crt` with the conventional
`/etc/ssl/cert.pem` symlink. Re-run the unchanged benchmark to obtain the public
score. The exact bundle hash is recorded in the JSON.

This workaround does not implement `IP_RECVERR` or validate general glibc DNS.
The host's resolver/hosts file, normal Firefox store, product defaults and
shared kernel source are unchanged. The transient resolved address is omitted
from public evidence. Runtime compatibility work should address the actual
socket-option/error-queue contract rather than advertising a no-op as support.

### Twelve-slot PDF follow-up

The 8192MiB/12-slot attempt passed Navigation but remained in multi-core
**PDF Renderer** without stage progress for more than 300 seconds. A process
state check reported 282.8% CPU and 5,271,888KiB RSS. These are point
observations; macOS process CPU accounting is not a direct measure of retired
guest instructions, and RSS is not unique guest physical storage. After a
one-second stack sample, this run was explicitly treated as diagnostic and
stopped at 714.34 seconds. It produced no score/upload.

The main thread mostly waited in `parallel_epoch`/`event_timeout`. Worker stacks
included `hv_vcpu_run`, page export/`Machine::allocate`/zeroing and
`trames_hvf_page`. HVF run frames include guest execution and kernel waits; this
sample does not attribute a dominant CPU cost. Source inspection found that
`host.c::table_for` linearly scans the address-space table list during each
page-table lookup. Indexed lookup and bulk physical-page admission are concrete
follow-up candidates; neither is claimed to explain all PDF latency without
further measurement. The four-slot result does not validate the twelve-slot
case.

[Filtered scores, provenance, hashes and diagnostics](mac-geekbench.json).
The previous code-validation results remain applicable because this checkpoint
changes only benchmark records and documentation.


## Demand allocation and compatibility follow-up (2026-10-28)

Mac source `017db6c` and runtime `3440534` replace the mapped-length RAM limit
with demand accounting. An unused VMA, mprotect, snapshot or futex metadata query
does not load file bytes or consume guest RAM quota. Actual file access and
hardware physical slots pay per reached page. Accessible descriptors still have
a separate 16 GiB representation ceiling and consume host metadata. Fork/COW
accounting remains conservative per VM rather than unique physical RSS/PSS.

The [backend contract](../architecture/execution-backends.md#physical-paging)
describes anonymous VM arenas, native-pager reclamation and counter semantics.
It removes checked disk preallocation and synchronous MS_SYNC from ordinary
RAM allocation/eviction. Per-space GPA indexes replace linear table ownership
searches. Migrating workers fill PTEs only from already resident, paid hardware
owners; untouched neighboring VMAs are not prefaulted or charged.

The exact local Linux workflow passed all 81 steps, reusing 21 matching passes.
Both runtime platform workflows, Mac CI, 780 common runtime macOS tests, 256 iPad
host tests, device/simulator Rust target checks, 25 signed HVF tests and the
signed application pressure test passed. The real 64 MiB paging/atomics/COW
fixture passed at 1/4 slots; its 16 MiB working-set run sampled about 48 MiB lower
RSS than the default run. Native-pager hints do not promise immediate RSS drops.

### Fixed working-set comparison

Three alternating repeats, 512 MiB, median milliseconds. Firefox and the local
Linux CI VM were stopped; no builds or samplers ran. Ordinary desktop apps
remained. Each page receives one 64-bit store, followed by eight checked read
passes. These are memory/translation phase timings, not general application or
CPU-throughput scores.

| Slots | First touch, before | First touch, after | Reuse, before | Reuse, after |
| --- | ---: | ---: | ---: | ---: |
| 1 | 445.059 | 350.341 | 5.268 | 12.890 |
| 4 | 488.241 | 321.357 | 108.968 | 24.930 |
| 12 | 505.578 | 353.564 | 68.380 | 54.752 |

Do not hide the short single-slot reuse regression. Extending the same read loop
from 8 to 800 passes gives 408.044 ms before and 413.934 ms after (1.4% slower).
The extra 792 passes cost about 0.51 ms/pass in either build, suggesting a fixed
phase/translation cost rather than a twofold steady-state memory slowdown. This
is an inference from timing, not an isolated stack attribution.

Reproduce with static AArch64-musl builds of `tests/fixtures/working-set.c` and
`tests/fixtures/cpu-features.c`, then run:

```sh
python3 tests/integration/mac-memory-compat.py \
  --runner target/release/trames-mac-run \
  --features /path/to/cpu-features --working-set /path/to/working-set \
  --mib 512 --repeats 3 --output target/memory-compat
```

### CPU and glibc DNS compatibility

HVF's configuration registers now supply runtime HWCAP/HWCAP2. This machine
reports `0x9017fb`/`0x6000`; real instructions validate AES, PMULL, SHA1/SHA2,
CRC32, LSE, FP16, RDM, dot-product, FHM, I8MM and BF16. SVE/SME and other
unsaved architectural state remain absent. The shared interface keeps the
interpreter/iPad's conservative default capabilities.

The original Ubuntu glibc getent now resolves the Geekbench domain through DNS,
with the earlier benchmark-only hosts record removed. Native sockets support a
scoped IP_RECVERR state for the runtime's system-DNS endpoint. Its error queue is
empty and MSG_ERRQUEUE returns EAGAIN without consuming queued DNS replies; the
session treats it as nonblocking. This is not general Linux ICMP error-queue
emulation: enabling it for non-DNS native UDP is rejected. DNS option/reply and
non-DNS rejection tests cover this boundary.


### Interval history and the complete DNS query path

The first-batch 12-slot/4096 MiB run completed all 32 workloads in 943.88 seconds,
then exited 255 during upload: libcurl could not resolve the result server.
One second of private thread-stack sampling was taken during its slow multi-core
portion; this is a diagnostic run, not a clean score. It produced no public score.
A point ps observation reported 9,499,424 KiB RSS and sample reported 7.8G
footprint. These host totals include metadata/aliases and are not a measurement
of unique guest physical pages. The 4 GiB guest setting remains a logical budget.

Sampling showed repeated full cached-PTE address-set construction and per-page
hardware-info checks. Source inspection found a concrete amplification: a change
larger than 4096 pages discarded hardware history, forcing every worker to scan
its full retained page table. Runtime `7730830` and Mac `36d62f8` use coalesced
interval history and intersect it with each worker's cached leaves. A 32 MiB
unused-VMA regression now checks zero unrelated leaf inspections; a real one-page
permission change still checks that page. Full reconciliation remains correct
when bounded history actually expires.

The upload failure exposed a second missing ABI operation: glibc's parallel
A/AAAA resolver uses **sendmmsg**, while IPv4-only getent uses sendto. Syscall 269
was returning ENOSYS. It now uses the existing sendmsg socket/memory path, returns
partial message counts, preserves post-send msg_len faults and SIGPIPE, and uses
the scheduler's existing blocking/restart/write-readiness paths. See the
[Linux implementation](https://github.com/torvalds/linux/blob/v6.12/net/socket.c).
Original glibc `getent ahosts` and `ahostsv4` now both succeed with no benchmark
hosts entry; the syscall trace records two messages sent successfully.

`runtime/tests/fixtures/sendmmsg.c` invokes the raw syscall so libc's own emulation
cannot bypass the test. Its datagrams, partial completion, output faults,
SIGPIPE/MSG_NOSIGNAL and zero-length FD validation match native Linux at 1/4
interpreter workers and also pass at 1/4 signed HVF slots. Mac signed acceptance
now has 26 HVF tests plus one application pressure test. Affected Linux/session,
macOS runtime/Mac, and iPad host/device/simulator checks passed. The earlier
81-step full Linux workflow is retained as the baseline; unchanged scopes were
reused rather than rerunning unrelated CPU/network/graphics acceptance.


### Clean four-slot result after the follow-up

[The complete final four-slot result](https://browser.geekbench.com/v6/cpu/19402105)
is Geekbench 6.7.1 Preview with the original benchmark/data, 4096 MiB and four
slots, source `36d62f8` / runtime `7730830`. It exited zero and uploaded normally
through real dual-stack glibc DNS. Firefox and the CI VM were stopped; no builds,
stack samplers or tracing ran. Thermal state stayed nominal and low-power mode
off before/after. Ordinary desktop apps remained. This is one completed run per
variant, not a statistically established distribution.

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Single-core score | 2421 | 3212 | +32.7% |
| Four-slot multi-core score | 2795 | 4857 | +73.8% |
| Complete run wall time | 462.30 s | 364.84 s | -21.1% |
| Multi/single ratio | 1.15x | 1.51x | Scaling still limited |
| PDF Renderer multi-core | 1009 | 3196 | 3.17x |
| Object Detection multi-core | 1600 | 6759 | 4.22x |
| Photo Filter multi-core | 284 | 1927 | 6.79x |

Not every component improved: single-core Clang changed 4664 to 4376 and Asset
Compression 3560 to 3478. Single-run variation and the bundled memory/capability
changes prevent attributing these differences, or the gains, to one mechanism.
The filtered JSON retains all 16 before/after component comparisons.

The single-core total is 74.9% of the earlier 4287 native 6.7.1 result. Native
multi-core used all 12 host cores, so its 18616 is not an equal-core overhead
comparison with this four-slot run. PDF, Photo Library, Text Processing and
Photo Filter still score lower in multi-core than single-core within Trames.
Do not call SMP scaling solved or infer whole-application native performance
from the former near-native CoreMark loop. Dense guest/snapshot/PTE metadata and
conservative fork accounting remain material memory/performance work.


### Clean twelve-slot completion and remaining scaling limit

[The final twelve-slot result](https://browser.geekbench.com/v6/cpu/19402164)
uses the exact same signed binary (`f24695cf82e56dbbbe2609a405e0fea4e492b39c4249da3f62d9953fc011cd46`),
4096 MiB, and no sampling/tracing/builds. It completed all 32 workloads, exited
zero and uploaded normally in 797.95 seconds, with nominal thermal state and
low-power mode off before/after. Navigation and PDF now finish at the original
4 GiB budget; the prior mapping-limit failure and upload blocker do not recur.
One completion validates this case, not long-term application stability.

| Metric/component | 4 slots | 12 slots |
| --- | ---: | ---: |
| Single-core score | 3212 | 3164 |
| Multi-core score | 4857 | 3971 |
| Complete run wall time | 364.84 s | 797.95 s |
| PDF Renderer multi-core | 3196 | 651 |
| Photo Library multi-core | 2949 | 715 |
| Text Processing multi-core | 2336 | 388 |
| Object Detection multi-core | 6759 | 3757 |
| Clang multi-core | 10808 | 20893 |
| Asset Compression multi-core | 8282 | 17291 |
| Ray Tracer multi-core | 11060 | 21771 |

Twelve slots score 18.2% below four slots overall. The single-core totals differ
by only 1.5%, while compute-heavy components gain roughly 2x and several
memory/lifecycle-heavy components collapse. This supports investigating shared
VM/metadata/checkpoint and mapping/reclamation work rather than assuming raw CPU
execution alone is the bottleneck; it is not a measured attribution of all lost
time. Interval history removes one demonstrated amplification but does not solve
all SMP costs. Dense page/snapshot/PTE descriptors, per-page mapping/retirement
operations and conservative fork copies/charges remain candidates for measured
follow-up. Do not increase product CPU defaults based on this result.

The twelve-slot multi score is 21.3% of the earlier native 18616 result with the
same host core count. Guest Linux versus native macOS builds and 4 GiB versus
24 GiB reported RAM still differ, so this ratio is whole-benchmark behavior under
these configurations, not the cost of an individual HVF instruction/trap.


### Twelve-slot scaling: bounded invalidation and linear arena addresses

The 3971 twelve-slot multi score above exposed two independent amplifiers.
An opt-in diagnostic build first narrowed physical-slot allocation: its pool
mutex had covered HVF mapping and 4 KiB initialization, serializing unrelated
CPUs. In a 12-thread, 1.5 GiB allocate/touch/read/unmap fixture, summed allocator
lock wait fell from 36.58 to 0.31 thread-seconds after moving those operations
outside the pool lock and adding weak thread-local slot hints. These are nested,
instrumented timings from one run each, not independent benchmark scores.

The original Geekbench binary/data then ran with that allocator change and
per-thread HVF diagnostics. It completed and uploaded in 862.74 s. Two 5 s stack
samples covered Navigation/PDF; this run is for attribution, not a clean score.
Approximate workload deltas from cumulative active-thread records showed:

| Multi workload | Cached leaves inspected | Synchronization thread CPU | Phase wall time |
| --- | ---: | ---: | ---: |
| PDF Renderer | 571 million | 70.2 s | 89.8 s |
| Photo Library | 1291 million | 160.0 s | 190.3 s |
| Text Processing | 299 million | 36.3 s | 45.8 s |

The bounded 32-update journal allowed a delayed CPU to fall back to a complete
cached-leaf walk. Work in that synchronization path also delayed joined mapping
checkpoints. Persistent metadata stamps per live 2 MiB chunk now let late CPUs
inspect leaves only in changed chunks, while the precise interval path remains
for current CPUs. Empty chunks leave no tombstones, byte owners or RAM charges.
Full snapshot replacement changes all stamps; permissions/storage/EOF admission
still uses the existing page checks. The signed 80-update regression verifies
zero unrelated inspections, 64 checks in one changed cached chunk and correct
read-only, unmap and fresh-byte behavior after address reuse. Software iPad Views
build neither chunk stamps nor the hardware journal.

A clean intermediate build (`8b4615c` / runtime `7943437`) scored 3137/5690 at
12 slots and 4096 MiB in 353.82 s:
[public intermediate result](https://browser.geekbench.com/v6/cpu/19402476).
This is +43.3% multi and -55.7% complete wall time versus the previous twelve-slot
completion. Its Photo Library and Photo Filter still scaled poorly; removing
full scans did not remove all physical mapping costs.

A subsequent 3 s sample of the bounded lifecycle fixture found worker waits on
the framework's mapping lock, with the owner in
`HvCore::HypervisorMemoryMap` vector insertion/erasure and `memmove`.
The Mac allocator now reserves a linear IPA range per existing anonymous arena,
matching its host-VA offset geometry. Table/import granules use a separate address
pool. This avoids scattering arena offsets among unrelated GPA allocations.
It reserves addresses only: each used 16 KiB granule still maps/unmaps on demand,
and free slots do not remain wired. On this OS/build, the instrumented fixture's
summed HVF data-map wall time fell from 23.63 to 8.46 thread-seconds; exact internal
framework behavior is not an API guarantee. Per-granule HVF calls remain.

Equal-total-1.5-GiB clean lifecycle comparisons used two rounds per thread,
4096 MiB, complete byte checks and matching checksums. Two runs per case used
opposite variant order; builds/profilers were stopped:

| Slots / MiB per thread | Before linear IPA, two wall times | After, two wall times |
| --- | ---: | ---: |
| 4 / 384 | 5415.69, 5432.72 ms | 2731.84, 2720.60 ms |
| 12 / 128 | 4553.26, 4470.03 ms | 2642.05, 2645.15 ms |

The affected local Mac workflow (boundaries, format, all-target/all-feature lint,
workspace tests, normal signed release) passed. Signed acceptance is now 29 HVF
tests plus one app pressure test. The new allocation tests race 12 host threads,
verify unique live slots and retained bytes, and check zero mapped/backing RAM
while weak TLS hints remain. Another test interleaves 256 private pages and table
allocations, checks linear GPAs, one-granule initial RAM, final release and reuse.
The shared-memory change passed 135 macOS memory tests, 784 native ARM Linux
runtime workspace tests and lints, iPad's 256 host tests/lints and both iOS target
checks. The prior full Linux workflow remains baseline evidence for unaffected
CPU, network and graphics acceptance; those suites were not repeated merely for
hardware-only invalidation metadata or documentation changes.


#### Final clean matched-source control

Final product source is `4de0946a4f007e51b19ba96530abf49c1a30e978`, runtime
`79434370422ff130117ed2066d4b22b49b79962c`. Both slot counts used the identical
normal signed executable, SHA-256
`fd5d26b330949b5c3db79bd92a7fef855e313f3fee544b6a9c07ddaf4cbce33d`,
original Geekbench 6.7.1 Preview instructions/data and 4096 MiB. The `diagnostics`
feature was compiled out. Firefox and Linux CI were stopped; no builds,
profilers or sampling ran. Thermal state was nominal and low-power off
before/after both runs. Each exited zero, completed 32 workloads and uploaded.
These are one clean run per configuration; ordinary desktop apps remained.

| Metric / multi component | Previous 4 | Previous 12 | Final 4 | Final 12 |
| --- | ---: | ---: | ---: | ---: |
| Single-core score | 3212 | 3164 | 3244 | 3241 |
| Multi-core score | 4857 | 3971 | 6001 | 7938 |
| Complete wall time | 364.84 s | 797.95 s | 306.31 s | 306.68 s |
| PDF Renderer | 3196 | 651 | 4855 | 5722 |
| Photo Library | 2949 | 715 | 7033 | 5254 |
| Text Processing | 2336 | 388 | 3606 | 2978 |
| Photo Filter | 1927 | 1778 | 3340 | 2613 |
| Clang | 10808 | 20893 | 10634 | 22403 |

Public final results: [four slots](https://browser.geekbench.com/v6/cpu/19402587),
[twelve slots](https://browser.geekbench.com/v6/cpu/19402563).
Twelve-slot multi improves 99.9% over the previous twelve-slot result, with
61.6% less complete wall time. PDF, Photo Library and Text Processing improve
8.79x, 7.35x and 7.68x respectively. The final twelve-slot total exceeds the
matched-source four-slot total by 32.3%, replacing the previous 18.2% regression.
Single-core totals differ by 0.1% between final slot counts. Four-slot multi
also improves 23.6% over its previous result.

Scaling is still incomplete: final twelve-slot Photo Library is 25.3% below
four slots, Text Processing 17.4% below, and Photo Filter 21.8% below. Clang,
Asset Compression and Ray Tracer scale about 1.9–2.2x from four to twelve slots;
several memory/lifecycle-heavy components do not. Per-granule HVF calls,
first-touch exits, joined shared-mm commits, dense metadata and conservative fork
copies/charges remain follow-up candidates, not measured exclusive attribution
of those remaining gaps. Product CPU defaults were not changed on this evidence.

Final twelve-slot single/multi are 75.6%/42.6% of the earlier native macOS
4287/18616 result. Linux versus macOS benchmark builds and reported 4/24 GiB
memory still differ; these are whole-run ratios, not an individual trap or
instruction overhead. Firefox remains closed; no interactive performance or
long-term release stability claim follows from the benchmark. The filtered
[JSON evidence](mac-geekbench.json) preserves all 16 component comparisons,
source/binary identities, diagnostics and scoped compatibility checks.


### Host-managed memory and removal of redundant resume/alias work

Product source is `da088d70a65b5fc4ceb8d723bb35a659a5e85c14`, shared runtime
`87a4f2036453226e6f8a779e6ae1616ba008461c`. The Mac default now uses host-managed memory,
without a product resident-byte quota. On this 24 GiB Mac, guest `/proc/meminfo`
reports 25,165,824 KiB total and live host free/available estimates; data/RSS/AS
rlimits and `/proc/self/limits` address-space limits are unlimited. Mach metadata
queries are confined to memory reporting, outside ordinary byte access. Explicit
`--memory-mib N` caps retain the prior guest-budget reports. A 16 MiB test passes
in auto mode and exits 137 with an 8 MiB cap. The normal PTY path reports the same
policy. No physical pool is reserved up front; existing used-granule mapping,
pressure reclamation and the macOS pager remain in use. The separate 16 GiB
accessible-VMA descriptor bound per address space and conservative fork/file
accounting remain; this does not claim unlimited sparse address metadata. iPad's
bounded default is unchanged.

An original-workload diagnostic baseline completed in 308.63 s.
Approximate per-workload cumulative thread deltas include 604,558 stage-one faults
in single Navigation and 525,416 in single Photo Filter. Multi Photo Library
spent 6.59 summed thread-seconds in HVF data mapping within 9.65 fault-handler
thread-seconds; multi Photo Filter spent 5.17 within 7.85. These nested, wall-time
thread sums overlap and are not an exclusive division of complete elapsed time.
A 3 s Photo Filter stack sample found framework mapping/unmapping work and
worker waits. A separate bounded original Navigation sample also found repeated
software CPU-window creation in snapshot preparation; its intentional timeout is
not a complete benchmark result.

Two mechanisms were removed. Hardware admission now keeps unused coordinator
software CPU aliases disabled across mapping changes and fork; cached byte
pointers are still invalidated and true COW is preserved. The EL1 restore prelude
performs pending TLB maintenance and register restoration in the same hypervisor
entry. A single leaf uses VA+ASID invalidation, with full-ASID maintenance for
roots, intermediate tables or multiple different leaves. Original page-table
barriers remain; ordinary guest accesses gain no new barriers or locks.

An equal-total-1.5-GiB lifecycle fixture, two recreate/touch/check/unmap rounds,
used two opposite-order runs per case and verified complete bytes/checksums.
Four-slot mean wall time fell from 2724.31 to 2558.13 ms (6.1%); twelve-slot mean
fell from 2624.55 to 2480.90 ms (5.5%), with visible twelve-slot variance. These
targeted results do not establish a comparable complete-application gain.

Both clean final runs used the identical normal signed executable, SHA-256
`ff823da9b98fe59da830aab72d063d7a2b0154d473c80302a9e9306b07ce56d4`, original Geekbench 6.7.1 Preview Linux instructions/data,
and twelve slots. Diagnostics were compiled out, Firefox and Linux CI were
stopped, and no builds/profilers/sampling ran. Both completed 32 workloads,
exited zero and uploaded with nominal thermal state and low-power off
before/after. Ordinary desktop apps remained; each policy has one clean sample.

| Metric / multi component | Prior 4 GiB | Current 4 GiB | Current host auto |
| --- | ---: | ---: | ---: |
| Single-core score | 3241 | 3238 | 3351 |
| Multi-core score | 7938 | 7797 | 8009 |
| Complete wall time | 306.68 s | 302.05 s | 299.78 s |
| Navigation | 2006 | 1861 | 1980 |
| PDF Renderer | 5722 | 5716 | 5826 |
| Photo Library | 5254 | 5348 | 5681 |
| Text Processing | 2978 | 3027 | 3131 |
| Photo Filter | 2613 | 2984 | 2901 |
| Clang | 22403 | 23700 | 23961 |
| Ray Tracer | 21245 | 21236 | 21721 |

Public results: [controlled 4 GiB](https://browser.geekbench.com/v6/cpu/19402951),
[default host memory](https://browser.geekbench.com/v6/cpu/19402993). The controlled total is
-0.1% single and
-1.8% multi versus the prior twelve-slot
result. There is no demonstrated overall Geekbench speedup from this round;
component changes go in both directions. Auto verifies the new memory policy
with 24 GiB reported RAM and is not a memory-matched optimization A/B.
First-touch exits, per-granule HVF map/unmap, joined shared-mm commits and dense
metadata remain follow-up priorities. Scores compare Linux and macOS benchmark
builds, not isolated instruction or trap costs. No interactive Firefox or
long-term release-stability claim follows from these runs.

Local validation passed: Mac workflow, 11 workspace tests, 30 signed HVF tests
and one pressure test; 788 macOS shared-runtime tests including 136 memory tests;
787 native ARM Linux runtime tests plus format/lint from a fresh archive of the
tracked runtime commit; 258 iPad host tests/lints and device/simulator target
checks. The two host-memory reporting unit tests run through iPad's host harness;
the standalone shared session crate disables its own lib-test target. Previously
passed CPU/network/graphics suites were reused for unchanged paths. Unrelated
dirty network and integration-test work was excluded and preserved. See the
[filtered JSON evidence](mac-geekbench.json) for all 16 components, policy proofs,
diagnostic boundaries, binary identities and scope details.


### Demand-fault continuation, publication and lazy arena registration

Final Mac source `01b808c4ea03e450c96266ae1f9438cba9f285a7` uses unchanged shared runtime
`87a4f2036453226e6f8a779e6ae1616ba008461c`. The original-workload diagnostic build on the preceding
source recorded 9,450,744 stage-one faults: 97.6% exported new storage,
97.5% were writes, and zero allocations were 96.1% of
faults. Repeated CPU translation of retained backing is therefore a minority of
faults. Approximate single Navigation deltas include 603,806 new-storage faults,
7.14 run/adapter thread-CPU seconds and 0.47 handler thread-CPU seconds. Run time
includes actual guest computation; this is not an exclusive overhead split.
Multi Photo Library/Filter still showed substantial HVF map wait. The profile
completed and uploaded in 305.81 s; it is not a clean score.

Immediate fault continuation now retains the paused EL1 context and original
absolute timer. New invalid-to-valid descriptors use DSB ISHST / ISB publication
without TLBI, including new table branches. Existing valid translations,
permissions, removals, root switches and ASID reuse retain invalidation; pending
maintenance is never downgraded. Complete materialized task state remains
available on all exits. No guest population or quota charge was added.
The first-touch fixture improved single-CPU total wall time about 9.6%, while
four/twelve-CPU improvements were about 3–4%. The intermediate clean twelve-slot
score was 3418/8083 in 297.18 s:
[public intermediate](https://browser.geekbench.com/v6/cpu/19403097).

A direct, bounded HVF probe established an additional mechanism: registering
128 MiB leaves zero data pages resident. Host/guest access then populated only
one 16 KiB granule, and withdrawing one granule kept its neighbor usable.
The allocator now registers each existing 1 MiB arena once, outside its mutex,
instead of registering every newly reached 16 KiB group. Registration is address
metadata; physical ownership, initialization, byte permission, host gates and
charges remain demand-driven. Used groups still unmap individually on reclaim
or final release; reaching a hole remaps just that group. Final arena teardown
withdraws remaining untouched registrations in contiguous runs. A failed
withdrawal preserves storage and address reservation. This behavior is measured
on the installed OS, rather than claimed as an Apple residency guarantee.

The isolated arena A/B used equal 1.5 GiB, two recreate rounds, opposite order,
matching values/checksums, no builds/profilers and the same explicit 4 GiB cap:

| Workload / CPUs | Before mean | Arena mean | Wall reduction |
| --- | ---: | ---: | ---: |
| All words / 4 | 2497.81 ms | 1857.33 ms | 25.6% |
| All words / 12 | 2289.90 ms | 2194.39 ms | 4.2% |
| First word/page / 4 | 2346.08 ms | 1786.76 ms | 23.8% |
| First word/page / 12 | 2227.62 ms | 2150.08 ms | 3.5% |

Final four/twelve-slot runs used identical normal signed bytes, SHA-256
`39651812d1483d1a769dfe330eaaa36b3f6ec06e6bca2f8fc6bdc7c9f8d999b1`, original Geekbench 6.7.1 Preview Linux instructions/data
and 4096 MiB. Diagnostics were compiled out; Firefox/CI were stopped and no
builds, samplers or probes ran. Both completed all 32 workloads, exited zero and
uploaded with nominal thermal state and low-power off before/after. Each slot
count has one clean sample; ordinary desktop apps remained.

| Metric / multi component | Before 12 | Fault fixes 12 | Final 4 | Final 12 |
| --- | ---: | ---: | ---: | ---: |
| Single score | 3238 | 3418 | 3425 | 3388 |
| Multi score | 7797 | 8083 | 6492 | 8181 |
| Complete wall | 302.05 s | 297.18 s | 291.31 s | 294.18 s |
| Navigation | 1861 | 2151 | 2137 | 2248 |
| PDF Renderer | 5716 | 6171 | 6315 | 6242 |
| Photo Library | 5348 | 6067 | 6944 | 6172 |
| Text Processing | 3027 | 3010 | 3757 | 3188 |
| Photo Filter | 2984 | 2874 | 4638 | 3423 |
| Clang | 23700 | 23406 | 10795 | 24294 |
| Ray Tracer | 21236 | 21454 | 10373 | 21715 |

Public final results: [four slots](https://browser.geekbench.com/v6/cpu/19403168), [twelve slots](https://browser.geekbench.com/v6/cpu/19403147).
Twelve-slot single/multi are +4.6% /
+4.9% versus the preceding clean 4 GiB twelve-slot
control. Twelve/four-slot multi ratio is 1.260 on the
same final binary. Relative to the earlier public 3241/7938 twelve-slot control,
final single/multi improve 4.5%/3.1%; that earlier control predates the host-memory
follow-up too. Whole-run single/multi are 79.0% /
43.9% of the earlier native macOS score; OS/binary
and reported 4/24 GiB RAM differ. This is not an individual trap-cost estimate.

The bounded instrumented dense fixture also confirms fewer actual mapping calls:
170,479 before these changes versus
44,883 afterward (73.7% fewer). Summed map wall time
fell from 8.45 to
2.51 thread-seconds. These overlap handler
time, include scheduler/reuse variation, and are not an exclusive speedup split.

Scaling remains incomplete. On the same final binary, twelve-slot Photo Library,
Text Processing and Photo Filter are below four slots by
11.1%,
15.1% and
26.2%.
Clang and Ray Tracer scale about 2.25x and 2.09x. The next structural investigation
should separate host handling of first-touch pages from joined shared-mm and
per-page metadata work; merely adding workers or raising RAM is insufficient.

Local Mac boundaries, format, all-target/all-feature lint, 11 workspace tests,
32 signed HVF tests, one app pressure test and normal signed release passed.
New acceptance checks one resident granule for a registered 1 MiB arena, hole
free/reuse, pressure withdrawal/page-in, final release, new table branches,
interrupted publication, complete registers and bounded timer reprogramming.
The unchanged shared runtime's preceding Linux/iPad validation is reused.
Per-page first-touch export, joined mm changes, dense metadata, the 16 GiB VMA
descriptor guard and conservative fork accounting remain. No Firefox recovery
or long-term stability claim follows from these CPU runs. Detailed filtered
evidence, all components and the probe records are in [JSON](mac-geekbench.json).

### Sparse anonymous VMAs and indexed retirement (2026-10-29)

Runtime `b230398dd102433599268a2fef4630d77a0a600c` replaces the dense metadata
cost above for large private anonymous mappings. Unreached accessible ranges
remain intervals; reached bytes and charges retain one canonical identity across
worker generations. Ordinary commits avoid duplicate coordinator descriptors.
Retirement and snapshot-index updates visit the affected stores/ranges, and
mmap/mprotect avoid constructing a process-wide region list. A 2,000-VMA guard
bounds a local mutation to at most two updated records. The common model serves
both Mac hardware execution and iPad software aliases. File/shared mappings keep
their independent descriptor bounds; first-touch exports remain per 4 KiB.

Five fresh shuffled trials measured untouched 1 GiB map time at 20.686 ms before
and 2.42 us after, with additional peak process RSS 50.58 / 0.11 MiB. Protect plus
refreeze fell from 108.607 ms to 10.08 us; cumulative additional peak RSS at that
stage fell from 295.31 to 0.27 MiB. These are process high-water marks, not unique
physical memory. The unchanged Linux memory fixture's single/twelve-slot wall
medians improved 2.9% / 2.0%; no whole-application gain is inferred from those
bounded trials.

The final normal signed release has SHA-256
`11dbfda9d79c39e5b5efa41a726d6adf2bed859b1ce6caf31880cd1647318076`.
Its clean twelve-slot, 4096 MiB Geekbench run completed all 32 workloads in
275.91 s and scored **3678 / 9134**, versus the preceding **3388 / 8181**
(single +8.6%, multi +11.6%). Navigation multi improved 2248 -> 7522 and Asset
Compression 13937 -> 16907. Photo Library and several image components did not
improve; Text Processing remains below its single-core score. Each revision has
one full sample. The earlier native macOS totals are 4287 / 18616; new Trames
totals are 85.8% / 49.1%, with the OS/binary/ISA/library and 4/24 GiB memory
differences still applying. Public results:
[new sparse implementation](https://browser.geekbench.com/v6/cpu/19404127),
[preceding control](https://browser.geekbench.com/v6/cpu/19403147).

A provisional implementation was deliberately stopped after multi-core PDF
exposed a cross-product traversal of VMA/retirement intervals. That sampled run
has no complete score and is excluded. The final indexed implementation passed
the same PDF stage and the complete suite without profiling. No CI VM, builds
or other probes ran during clean measurements; thermal state was nominal and
low-power mode off before/after. Firefox stayed closed.

Local common macOS and native ARM64 Linux workflows passed. Memory coverage has
141 lib tests plus the allocation and translation integration tests. Mac
boundaries, format/lint, workspace tests, 32 signed HVF tests, one pressure test
and normal signed release passed. iPad workspace checks plus device and simulator
target compilation passed. Checks use the intended tracked runtime sources and
preserve unrelated network/integration-test work. Prior unchanged acceptance is
reused. Native range backing/block translations and joined shared-mm changes
remain follow-up work. Full component scores and limits are in
[mac-geekbench.json](mac-geekbench.json); bounded raw measurements are in
[mac-memory-model-probes.json](mac-memory-model-probes.json).


### Native anonymous extents and live mmap publication (2026-10-29)

Common runtime `68552f7bd3b9534fe15fe1fd4a5c4e22b7415998`, Mac source
`4ab04b4f90b9e391d0dca4eb710f6d23d90d2f5a`, normal signed runner
`4b964e1da487ac2dd5d7c6bebf78bad531a6360a9c63b1f8312a0314fbfac683`. Host-managed
uniform private-anonymous RW extents now use lazy native 2 MiB backing and L2
blocks, eliminating ordinary per-4KiB data export exits. Actual pages remain
host-demand-paged. Pure new RW anonymous mmap publishes through an insertion-only
common index without pausing same-mm peers; destructive changes retain their joined
boundary. The shared interpreter regression uses the same publication model,
preserving iPad compatibility.

The retained five fresh shuffled lifecycle trials per configuration validate
matching values. They use runtime `e5a2523` / Mac `73ebdb1`; final
memory/mmap-addition/HVF source is unchanged (diff verified), and their exact
earlier executable hashes remain in the JSON evidence:

| Memory policy / workload | Before median | After median | After / before |
| --- | ---: | ---: | ---: |
| Host-managed, 1 CPU, 256 MiB, one word per 4 KiB | 168.877 ms | 19.520 ms | 0.116 |
| Host-managed, 12 CPUs, 128 MiB each, full stores/reads | 2171.105 ms | 134.185 ms | 0.062 |
| Host-managed, 12 CPUs, 64 MiB each, one word per 4 KiB | 927.642 ms | 53.474 ms | 0.058 |
| Strict 4096 MiB, 1 CPU, same sparse workload | 167.916 ms | 170.494 ms | 1.015 |
| Strict 4096 MiB, 12 CPUs, same dense workload | 2271.118 ms | 2174.117 ms | 0.957 |
| Strict 4096 MiB, 12 CPUs, same sparse workload | 906.434 ms | 956.094 ms | 1.055 |

Default-policy speedups are bounded to this memory workload. Strict caps and
explicit working-set targets retain page admission. Strict control changes are
+1.54% / -4.27% / +5.48%; no stable strict-cap gain is claimed. A partial live
extent can retain 2 MiB host backing; actual host pages are 16 KiB and need not be
physically contiguous. Sparse fork includes compressed and paged-out contents;
metadata query failure falls back conservatively. VM replacement preserves canonical
bytes/accounting while old owners retire.

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
assessed in [the complete component evidence](mac-geekbench.json). The new totals
are 90.6% / 66.1% of the earlier [native macOS
result](https://browser.geekbench.com/v6/cpu/19400588). The latter also reports 24
GiB but differs in OS, benchmark binary, library/ISA dispatch and date; this is a
platform comparison, not isolated HVF overhead.

Local verification includes final memory regressions/lints on macOS and native ARM64
Linux (148 lib tests plus six integration tests), 39 signed HVF tests, one app
pressure test, normal signed release and both iOS compile targets. The final
proc/UUID fix passes 202 process conformance cases (three ignored), 325 VFS tests on
both Mac/Linux, affected lint and both iOS targets; memory/provider checks are
reused unchanged. Earlier passing workflow results are retained for unchanged
components. No CI VM, builds or samplers ran during the clean measurements;
diagnostics were compiled out, thermal state nominal and low-power off before/after.
Firefox stayed closed. English ownership/limits and filtered evidence are in
[execution
backends](../architecture/execution-backends.md#native-anonymous-ranges-and-live-mmap-publication-2026-10-29),
[memory evidence](mac-memory-model-probes.json) and [all Geekbench
components](mac-geekbench.json).
