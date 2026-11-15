# Private storage sync optimization — 2026-11-15

Private data-only synchronization improves **4.54×**, from 55.93 to 254.15 IOPS,
matching the shared-directory path's retained full-flush behavior. Full fsync
improves modestly, from 55.53 to 62.39 IOPS (+12.3% raw; about +5.4% after
normalizing to the changed native full-sync reference). It remains a bottleneck.

Runtime: `091dfae55520d00d9dfb0445ba0f024f98b664f3`, baseline `60338f564cea811450a743e1a613e2135cbeef6d`.
The common implementation is paired with both Mac and iPad product pins.

| Workload (4 KiB write, sync after every write) | Private before | Private after | Shared after | Native Mac after |
|---|---:|---:|---:|---:|
| fsync | 55.53 IOPS | 62.39 IOPS | 253.95 IOPS | 285.03 IOPS |
| fdatasync | 55.93 IOPS | 254.15 IOPS | 254.10 IOPS | 66,055.79 IOPS* |

*Native macOS fio calls fdatasync. Trames still uses Apple Rust sync_data's
stronger F_FULLFSYNC. This diagnostic therefore must not be presented as an
apples-to-apples forwarding-overhead ratio. The optimization removes redundant
namespace work without weakening the established file-data flush.

## Implementation

The version 4 manifest stores identity/permissions/timestamps/attributes and
paths, not ordinary file length. A private fdatasync now synchronizes the original
object, whose host length is recovered at reopen, without replacing the entire
namespace just for timestamps. Original descriptor execution, mapped writeback
and retained disk jobs share this rule. Overlay copy-up keeps its checkpoint.

Store-local generations track object creation/final deletion and manifest
publication. A successful directory flush covers only its captured generation.
Already durable directories require no additional physical flush. Failure after
rename keeps durability pending, so an explicit sync retries and reports errors
without rolling back a committed namespace. Generation tracking is absent from
ordinary byte reads/writes; no coordinator or service thread was introduced.

[Common runtime design](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/private-storage-sync.md)
describes ordering and the remaining full-manifest costs.

## Verification

- Full Runtime workflow locally on macOS: 7/7 steps, 1,600 tests passed, 31
  existing ignored tests. All-target/all-feature clippy with warnings denied,
  release build and memory-reclamation example passed.
- Full Mac workflow locally: 13/13 steps. Unchanged native UI checks reused;
  affected Rust/source-boundary and build checks rerun.
- Physical-iPad Rust target: VFS and complete product FFI both compile.
- Persistence regression covers hardlinks, growth/reopen, asynchronous resource
  retention, mapped dirty pages, manifest rejection, post-rename durability
  failure/retry and process exit without VFS shutdown. This is not a hardware
  power-cut validation.
- Reused the identical fio 3.43 binaries and parameters from the
  [baseline](mac-storage-fio-20261115.md), adding `--cases write-4k-fsync
  write-4k-fdatasync`: 18 measurements, three shuffled repeats, 5 seconds plus
  1 second ramp. Exact 4,096-byte counts, successful jobs and runtimes verified.
- A separate 5-second wall-stack sample confirms full host flushes dominate
  active sync paths, with manifest serialization also visible. Idle Hypervisor
  and other waiting threads are not counted as CPU overhead.

[Machine-readable results](mac-storage-sync-20261115.json) contain the complete
comparison, records, hashes and limits. Raw fio JSON/logs remain under
`target/storage-sync-20261115/`. Benchmark files were removed and its isolated
service stopped. Existing user services and Stable remain running. About 27 GiB
of disk space remains; build caches were reused.

The Mac Dev application has been updated. Existing live environments use the new
runtime on their next restart. The iPad source pin is updated and compiled; no new
physical-device installation is included in this Mac optimization task.

## Next significant work

Full fsync still serializes and atomically replaces the whole manifest and runs
separate file/manifest/directory durability operations. An incremental metadata
commit design or a proven ordered barrier batch can reduce that cost further.
It requires updating all recovery and interoperation consumers if the format
changes; simply dropping flushes would change the durability guarantee.
