# File-specific private-store synchronization — 2026-11-15

Private full `fsync` improves **21.0%**, from 62.39 to 75.46 IOPS, by reusing the
committed manifest snapshot instead of reconstructing the namespace for each
file sync. The shared-directory control is unchanged. The cumulative improvement
from the original 55.53 IOPS baseline is 35.9%, reaching 75.46 IOPS. Full private
synchronization remains slower than shared and native files because its
persistent metadata protocol still writes
and synchronizes a separate manifest.

Runtime: `0388a05e3191b18de3ad3183a7cf42270686c494`, previous
`091dfae55520d00d9dfb0445ba0f024f98b664f3`. Mac and iPad use the same common
implementation and paired runtime pins.

| 4 KiB write, full fsync after every write | Previous median | Current median | Change |
|---|---:|---:|---:|
| Private store | 62.39 IOPS | 75.46 IOPS | +21.0% |
| Shared directory | 253.95 IOPS | 253.80 IOPS | −0.06% |
| Native macOS | 285.03 IOPS | 278.74 IOPS | −2.2% |

Private trials were 74.47, 76.66 and 75.46 IOPS. Normalizing to the native control
would give a 23.7% improvement, but the table reports the directly measured
change. Data-only sync was not repeated: its unchanged path's earlier 254.15
IOPS result remains a historical measurement in the
[previous acceptance report](mac-storage-sync-20261115.md).

## Mechanism and boundaries

The existing per-store checkpoint mutex now owns the last committed serialized
v4 manifest and a compact sorted inode/header-offset vector. A regular-file
sync patches only the original inode's fixed 64-byte metadata header. It avoids
walking paths, rebuilding inode/attribute tables and borrowing unrelated inode
metadata. Concurrent file syncs update the latest committed snapshot under the
original transaction lock. Unchanged metadata skips publication; file-data
synchronization and any pending directory durability retry still run.

Namespace and attribute transactions retain a complete rebuild, replacing the
snapshot and index only after successful publication. Hardlinks, renames and
replacement therefore cannot leave stale positions. Retained open files keep
their original inode, rather than targeting a reused pathname or descriptor.
Failure before rename restores both cached header and checksum. The existing
post-rename directory generation preserves retry/error semantics.

This is incremental **CPU-side preparation**. Dirty metadata still recalculates
the whole-manifest checksum and writes the complete manifest with the original
atomic replacement and full file/manifest/directory flush sequence. The format
and durability guarantee are unchanged. The cache consumes actual manifest
bytes plus approximately 16 bytes per indexed inode on 64-bit hosts. Ordinary
byte reads/writes gain no counter, synchronization barrier, lock or coordinator;
no new worker/service thread was introduced.

[Common runtime design](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/private-storage-sync.md)
records the ordering and remaining costs.

## Validation

- Complete Runtime workflow locally on macOS: **7/7 steps, 1,604 tests passed,
  31 existing ignored tests**; all-feature/all-target clippy, release build and
  reclamation example passed. This does not claim a Linux CI matrix run.
- Complete Mac workflow locally: **13/13 steps**. Affected source/boundary,
  Rust and packaging checks reran; unchanged native UI checks reused passing
  input hashes.
- Complete iPad product FFI compiled for `aarch64-apple-ios`. No physical-iPad
  performance measurement or new IPA installation is included in this task.
- New regressions cover a target sync completing with unrelated inode metadata
  locked, four concurrent retained-inode sync jobs merging metadata,
  hardlinks/xattrs/modes across publication failure/retry, clean-sync reuse,
  orphan/replacement lifetime and rename/index rebuilding. Existing reopen,
  process-exit and post-rename durability regressions also passed.
- Identical fio 3.43 binaries to the previous run (`--disable-native
  --disable-shm`), psync depth 1, one job, buffered I/O, exact 4,096-byte writes,
  a 256 MiB file per variant, three shuffled repeats of 5 seconds plus 1 second
  ramp, and full fsync after every write. All nine jobs succeeded.
- Host and guest use the same APFS storage. These are buffered explicit-sync
  measurements, not raw SSD throughput. No hardware power-cut test was run.

[Machine-readable results](mac-storage-inode-20261115.json) include all nine
records, source/binary hashes, parameters and comparisons. Runtime and Mac CI
records verify that the tested source hashes match the final commit. The
benchmark candidate was compiled before updating its product revision string;
the installed Dev application was rebuilt with the final paired pin.

Mac Dev is installed with the embedded Vulkan driver. Running environments pick
up the new runtime on their next restart. Stable and existing user services were
not restarted. The iPad source pin is paired and compile-checked. Benchmark data
files were removed and the isolated benchmark service stopped; about 25 GiB of
free disk space remains and existing build caches were reused.

Further gains require reducing whole-manifest checksum/write work or proving an
ordered durability batch. Removing flushes without establishing equivalent
ordering would change the guarantee and is outside this change.
