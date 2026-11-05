# Original thread-group exit completion — 2026-11-05

Runtime `004ab49f8629af1259056249252b76519d4df334` retains logical membership through resource/View cleanup and
uses the original group for final status, host/family completion and process-lock
release. Last ordinary exit supplies status unless explicit group termination
fixed it earlier. Retired members cannot be recreated by late owner updates.

Source CPU collection does not replay accepted lifecycle effects. It preserves
owner-local ancillary-rights and detached-mount collection without retiring
original descriptors again. The occupied-native-context and detached-directory
regressions both have failing old-behavior controls. A stock previous Dev also
fails the new actual Linux final-status fixture.

Apple: 311 process, 55 shared kernel and 507 hostfs-enabled VFS checks pass. Native
ARM Linux validates the full group semantic change and final collector's three
affected cases. iPad device-target compilation and strict affected Clippy pass.
The paired final binaries pass 20 lifecycle cases (HVF 1/4/12, interpreter 1/4),
native GPU/video, real H.264/Vulkan sharing, 30 Go GC starts and eight isolated
12-CPU Docker lifecycle rounds. No physical iPad or microphone run is claimed.

| 128 two-thread groups | Previous Dev | Current Dev | Native ARM Linux (6 CPUs) |
| --- | ---: | ---: | ---: |
| 1 CPU | 16.704 ms | 16.472 ms | 7.661 ms |
| 4 CPUs | 29.559 ms | 29.732 ms | 7.661 ms |
| 12 CPUs | 29.212 ms | 29.275 ms | 7.661 ms |

Seven counterbalanced pairs use one ELF and include joined fork/mm/final-exit
work. The Linux reference is the existing 6-CPU/8-GB hardware-backed test instance,
not a native macOS syscall equivalent. Timings exclude builds and that instance.
These are regression controls, not a Firefox/general CPU speedup claim.

Last/group/default-fatal entry, final CPU/Memory custody, vfork, namespace-init
shutdown completion, robust futex and actual owner controls remain connected
work. This batch introduces no service thread, polling bus or new admission gate.

Dev was installed after acceptance. Stable 1.5.3 and business services retain
their previous binary; no Techo frontend was activated.

[Shared architecture](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/coordination-boundaries.md) ·
[Full source and sample evidence](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/thread-retirement-20261105.json)
