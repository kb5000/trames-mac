# Mac Linux compatibility and paired CoreMark

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
