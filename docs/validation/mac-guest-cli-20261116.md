# Shared guest and Mac owner CLI — 2026-11-16

Runtime: `77543a5b4d31ac0a7f144ed1f343e1411053142e`.

## Behavior

One static ARM64 musl command is injected into Linux (`/usr/local/bin/trames`
and `/usr/bin/trames`) and Android (`/system/bin/trames`). Human help, English/
Chinese output, JSON, completion, status/doctor and a read-only guest permission
interface share their implementation. Mac owner commands use `-e` and can
allow/deny permissions or set defaults for future environments. Owner creation
and restore use a fresh UUID-bound grant; explicit denials and read-only share
restrictions stay intact. Restoring never imports archived native share paths.

File copies work in both directions with file permission alone. Guest aliases
use existing hostfs views; host copies use the running guest's original exec
pipes/VFS or its offline locked VFS. Bulk bytes never travel in control JSON.
Live copies use a scoped per-copy archive encoder/decoder, not a resident
service or coordinator. Rename avoids a second bulk data copy for new entries.
Explicit file deadlines and recursive/no-clobber copies are supported; owner
interactive copy is rejected with usage guidance.

Host command streams preserve literal argv, binary stdin/stdout/stderr, EOF and
exit status. Accepted streams have no fixed total timeout. Original external
producer readiness remains active without a timer. Cancellation cleans the
owned process group even after the leader has exited while descendants retain
pipes. Delegated CLI cannot mutate grants/select another environment or stop
its own command channel. Full host-exec retains the host user's access.

## Validation

- Full local shared-runtime CI: 8/8 steps, 1,612 passed tests.
- Full local Mac CI: 14/14 steps (static checks, 37 passed unit tests,
  native task/window checks, both channels built and packaged). Entitlement-
  gated tests remain excluded from ordinary cargo test; real-HVF acceptance
  below uses the signed Dev runner.
- Real-HVF CLI acceptance: 41 operations, 256 KiB binary command
  input/output, separate stderr, nonzero exit, literal arguments, deadlines,
  descendant cancellation, independent files/mount grants, and live/offline
  bidirectional file/tree copies.
- Existing environment lifecycle/restore acceptance: 38 operations.
- Existing manager acceptance: concurrent creation, live guest saves, strict
  boolean serialization, revision conflicts and unchanged authorization on save.
- Existing generic-rootfs/CLI acceptance, including the injected command on
  custom Debian/glibc, and idempotent PATH installation.
- iPad embedding: 15 FFI tests, physical-target Rust cross-check and Swift
  syntax checks. Shared static command help also executed on the interpreter.
  No physical iPad installation or UI acceptance is claimed this round.
- Workflow/project-layout validation: 3 tests.

For one cached 64 MiB file (content hashes checked), native host copy took
0.0192 s; live owner upload 0.336 s
and download 0.301 s. This is a cached end-to-end copy
comparison, not a durable-storage/fsync benchmark. Ordinary existing file I/O
paths are unchanged.

## Deployment and limits

Deployment is limited to Mac Dev; Stable and existing running guests keep
running. An existing session receives the new injected binary/control metadata
on its next normal restart. Existing permissions are not silently widened.

The iPad adapter uses the original security-scoped Files provider, including
its single-file/1 GiB limit. Native folder IDs are share aliases; unsupported
recursive/no-clobber provider operations fail explicitly. The owner explicitly
authorized removing the old per-request Trames consent sheet; environment
policy, OS privacy and Files folder authorization decide access. iPad arbitrary
host-exec, delegated Mac CLI, dynamic mounts and port administration remain
unsupported. The shared design and user guides describe these platform limits.
