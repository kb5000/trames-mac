# Trames Mac 1.5.2 acceptance (2026-11-04)

This stable update combines the retained event-wait, POSIX/real interval timer
and shared signal migrations after 1.5.1 with two independent runc startup fixes.

- Namespace copies no longer compete for an erroneous global 128-mount quota.
  The per-namespace mount-table bound and resource retirement remain.
- Cross-process `prlimit64` resolves namespace-visible PID/TID targets, checks
  caller authority and updates the target process limits shared by its threads.
  Fork copies remain independent and target FD allocation enforces the change.
- Ordinary admitted event waits, timers and signal operations run on existing
  execution workers. Lifecycle/default stop/termination and structural policy
  boundaries still remain; this release does not claim all coordinator work is
  removed. The limit correction preserves its existing joined policy boundary.

Runtime: `c973060c485df0ab88352fe35e8ee9cce07646a3`.
Previous stable runtime: `d828821b907f8a0d84df01eb934b778858ef873f`.

## Local validation

All repository Mac workflow shell steps pass locally: boundary and formatting
checks, strict workspace Clippy, product tests, script syntax, native task-stream
acceptance, stable/dev builds and package validation. Packaging includes the
unchanged native-driver corresponding sources in addition to paired Git source
exports. Stable/dev install isolation and structured exec/multiple-shell
integration tests pass in temporary environments.

Apple and native ARM Linux each pass 290 process/kernel-conformance tests (three
ignored) and 17 kernel unit tests. Strict shared checks and the physical-iPad
FFI target compile pass. Unchanged VFS/ABI, graphics and network evidence from
the validated preceding migrations is reused.

A static-musl probe fails on the previous Dev runtime with ESRCH and passes on
native ARM Linux, HVF at 1/4/12 CPUs and the shared CPU interpreter at 1/4 CPUs.
It checks parent-to-child and parent-to-thread updates, actual EMFILE, shared
thread limits, caller independence and a reaped target. Original runc
1.1.15+ds1-2+b4 also passes 72 concurrent starts with its default explicit nofile
limit (1024/1024): Dev at 4/12 CPUs and the stable binary at 12 CPUs. The isolated
pressure test uses 32 empty tmpfs mounts and three rounds of eight starts; no
production application data or paid APIs are used.

[Source-bound follow-up evidence](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/prlimit-release-1.5.2-20261104.json)
· [Mount failure analysis](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/guest-containers.md#concurrent-runc-bootstrap-and-mount-resource-tracking-2026-11-04)
· [Current coordination boundaries](../../runtime/docs/architecture/coordination-boundaries.md).

## Distribution and upgrade

Stable and Dev are separate locally signed, unnotarized ARM64 apps, retaining
the selected distribution method. Both installed 1.5.2 bundles pass signature
verification and contain the committed logo. Their data registries stay outside
the application and are not replaced by installation.

Existing Debian/Techo and the separate Docker HTTP service remained healthy
before and after installation. They were not restarted: a running environment
continues on its old mapped runtime until it is stopped and started again.
The same shared revision is pinned by iPad and passes its compile check; no new
iPad IPA release or physical-device test is claimed here.
