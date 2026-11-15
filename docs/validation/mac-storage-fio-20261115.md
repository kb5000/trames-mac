# Mac storage fio acceptance — 2026-11-15

Cached reads and writes retain substantial overhead compared with native macOS.
The clearest persistence gap is the private store: about 56 full-sync writes/s
versus 267/s on the host and 258/s through a shared directory. This is an I/O
comparison, not a physical SSD throughput or a CPU-time profile.

## Results

Apple M6, 12 logical CPUs, 24 GiB RAM, macOS 27.0.1, APFS. Trames Dev 1.5.5,
product `b67f8778ce76`, runtime `60338f564cea`.
Medians of three shuffled repetitions; 72 completed measurements.

| Workload | Unit | Native Mac | Private | Shared | Private/host | Shared/host |
|---|---|---:|---:|---:|---:|---:|
| Sequential read, 1 MiB | GiB/s | 20.10 | 10.34 | 10.38 | 51.5% | 51.7% |
| Sequential write + final full sync, 1 MiB | GiB/s | 8.72 | 3.74 | 4.67 | 42.8% | 53.5% |
| Random read, 4 KiB, 1 job | kIOPS | 1,270.21 | 466.43 | 404.58 | 36.7% | 31.9% |
| Random write, 4 KiB, 1 job | kIOPS | 589.85 | 316.71 | 285.42 | 53.7% | 48.4% |
| Random read, 4 KiB, 4 jobs | kIOPS | 3,611.64 | 1,237.92 | 1,084.64 | 34.3% | 30.0% |
| Random write, 4 KiB, 4 jobs | kIOPS | 827.87 | 627.48 | 600.87 | 75.8% | 72.6% |
| Write 4 KiB + full sync every write | IOPS | 267.35 | 55.53 | 257.95 | 20.8% | 96.5% |


Sequential synchronized write results vary considerably: native 4.36–9.42 GiB/s,
private 3.11–6.89 GiB/s, shared 4.21–6.49 GiB/s. These short buffered jobs
include final-drain effects; do not interpret the median as a sustained SSD rate.

## Method and compatibility

- Both binaries use unmodified upstream fio 3.43, configured with
  `./configure --disable-native --disable-shm`. The Linux binary was built in the existing Dev Debian after installing make;
  its dedicated temporary build directory was cleaned afterward. Guest default `shmget` returned
  ENOSYS before switching to fio's official shared-mmap build option.
- Common engine `psync`, depth 1, thread mode, clock_gettime, no cache
  invalidation or preallocation, explicit `kb_base=1000` for true IEC sizes.
  Every output byte count was verified against the requested block size.
- One 256 MiB file for serial jobs; four separate 64 MiB files for 4-job
  tests. Same APFS volume. Five measured seconds plus one second warm-up.
- Every write job uses end_fsync. Full-sync tests also use fsync=1. Native
  fio detects F_FULLFSYNC, matching Apple Rust sync_all used by Trames.
- Native O_DIRECT/F_NOCACHE probe succeeded. Private and shared O_DIRECT
  probes returned EINVAL. All compared throughput is explicitly buffered.
- An isolated TRAMES_HOME contains a minimal Debian image and only a dedicated
  benchmark share. Existing user environments stay running. CPU utilization
  fields in guest fio are zero and are not suitable for CPU-cost comparisons.
- The benchmark dataset was removed and its service stopped. Disk free space
  returned to approximately 31 GiB; no broad cleanup or rebuild was required.

## Synchronization diagnostic

`fdatasync=1` produced about 65,000 writes/s natively, 56/s privately and 255/s
through the shared directory. This row is deliberately excluded from the main
ratio table: macOS fio calls native fdatasync, while Apple Rust sync_data uses
F_FULLFSYNC. The guest therefore applies a stronger durability operation. This
is a meaningful compatibility/performance issue for data-sync-heavy Linux apps,
not a fair estimate of raw syscall forwarding overhead.

Code inspection also shows that linked private file synchronization can checkpoint
the namespace manifest, synchronize its objects directory, atomically replace
the manifest and synchronize the store directory. Shared files bypass that
private namespace transaction. This is a likely contributor to the roughly
4.8× private full-sync gap; this run does not attribute exact time to each step.

Useful follow-up work is to measure/reduce repeated namespace publication and
full-device flushes while preserving ordering and crash durability, then inspect
cached 4 KiB read/copy costs. No runtime implementation was changed in this run.

## Reproduction and evidence

[Machine-readable results](mac-storage-fio-20261115.json) retain every measured
record, ranges, binary hashes, source checksum and direct-I/O probe errors.
Full fio JSON and compiler logs remain in the local run directory under
`target/fio-storage-20261115/`; preliminary unit-check runs are excluded.

```sh
python3 scripts/benchmark-storage-fio.py \
  --runner /absolute/path/to/trames-dev \
  --native /absolute/path/to/mac-fio-3.43 \
  --guest /absolute/path/to/linux-arm64-fio-3.43 \
  --rootfs /absolute/path/to/cached-debian.tar.gz \
  --output /absolute/path/to/disposable-benchmark-dir \
  --repeats 3 --runtime 5
```

Upstream references: [fio unit semantics and engines](https://github.com/axboe/fio/blob/fio-3.43/HOWTO.rst),
[fio sync operations](https://github.com/axboe/fio/blob/fio-3.43/io_u.c),
[Apple Rust synchronization](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/unix.rs).
