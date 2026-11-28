# Shared storage and pathname runtime pin — 2026-11-28

Runtime: `03d21f4f2ac7aa7be00c7827afad4a3b2ad8ec29`.

Private metadata uses incremental WAL plus bounded periodic checkpoints. The VFS
name cache keeps live inode metadata/permissions. Both paths apply to this product.
Compression/crypto thunks follow the existing option; the hardware execution
backend retains its disabled default and original guest library instructions.
No UI or new setting is added.

Validation on Apple ARM64 Mac host:

- `cargo check --offline --locked -p trames-mac`: passed.
- `python3 scripts/check-execution-boundaries.py`: passed against the clean, actual runtime pin.
- `python3 scripts/prepare-third-party-notices.py --check`: passed. The seven new
  native/build dependencies include reviewed original upstream license texts.

The existing import/backup paths open the store through VFS before copying its checkpoint; startup recovery compacts the WAL under the retained store lock.

Shared Apple/Linux storage tests, computation ABI matrix, WAL recovery and limits
are recorded in [the runtime validation](../../runtime/docs/validation/storage-compute-cache-20261128.md).
No new hardware guest acceptance or end-to-end application speed measurement is
claimed by these product compilation checks.
