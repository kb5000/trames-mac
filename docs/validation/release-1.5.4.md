# Trames Mac 1.5.4 (2026-11-07)

This stable release consolidates the original-resource coordinator work through
Unix ancillary closure and fixes a release-gate Docker startup regression.
The shared runtime is pinned in runtime-version.json and shared with iPad.

Task/image lifecycle, mount/namespace/credential administration, proc consumers,
seccomp, BPF, memory barriers and entropy use existing execution workers and
original shared objects. The FD/network RPC and Unix ancillary owner-ledger/message
handoff categories are removed. Explicit tracing, excluded resources, affected
memory/backing checkpoints and actual host/session lifecycle remain boundaries.

A returned cohort could leave accepted ELF loads in the existing disk pool while
no guest CPU/socket/timer was runnable. The joined idle predicate omitted them
and reported a false Deadlock. Pending image jobs now keep the original driver
alive and use its existing completion unpark token. There is no new coordinator,
service thread, busy loop or normal instruction/syscall check.

The deterministic immediate/delayed completion regression, 384 process cases on
Mac and ARM64 Linux, both complete runtime workflows and strict checks pass.
The corrected real 12-CPU Docker binary completes eight full lifecycle cycles.
Detailed final artifact, signing, product and installation evidence is published
with the 1.5.4 release and the paired iPad validation record.

Mac distribution remains locally signed and unnotarized. Stable and Dev retain
independent identities and storage. Installing an app preserves environments;
running environments adopt the new runtime on their next restart.


The proc fdinfo renderer now observes the committed original offset without
reentering its active read transaction. The original file mutex and pending-read
invariant remain intact; the published scalar uses relaxed atomic access.
Full 1/4-CPU systemd boot, services, journal, notify, stop/restart, backup/restore
and real concurrent CLI acceptance pass. Restart readiness checks the actual
console listener; a stale pathname cannot produce a false ready result.
The native snapshot-cache regression also retains its original physical/code/data
assertions while allowing a legitimate unrelated global backing invalidation.
