# Shared storage and pathname runtime pin — 2026-11-28

Runtime: `8fee46855ab3e092f27fbc654b9f0d9ef413275a`.

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

The complete-workspace follow-up also pins CLI `bd22dbf1b6f9875d2298633467bf81f1516b6f5b`. Product compilation
was repeated with the final shared login/ALSA helper and network accessor changes.
Shared distro unit tests passed 47 cases each on Apple and Linux. CLI code is
unchanged by its documentation-only commit.
