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
