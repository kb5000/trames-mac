# Docker recovery and native GPU publication, 2026-11-13

Runtime `86dfa2cbdcdb7dfd8f552ee92237c505b0f3068b`; Mac 1.5.5.
Machine-readable evidence is in [the paired record](mac-docker-gpu-20261113.json).

## Docker regressions

The stable store was copied offline after explicitly stopping its independent
Docker environment. The Debian business environment remained running. Every
reproduction and destructive lifecycle check uses a disposable copy of that
647 MiB store, including its original systemd, Docker 26.1.5 and runc 1.1.15.

Source-dispatched blocking FIFO open returned a reserved, still-hidden descriptor
before the original endpoint rendezvous completed. runc then reported EBADF on
`exec.fifo`. Ordinary openat/openat2, native file work and io_uring now retain the
same original endpoint, subscribe before checking completion, and release the
execution slot while waiting. Cancellation removes only that exact reserved OFD;
concurrent close/reuse cannot close a replacement descriptor. No coordinator or
new service thread is involved. Tests cover both open operations, peer open/close,
actual bytes/EOF, cancellation, close_range/reuse, and io_uring completion order.

After first startup succeeded, daemon restart exposed a second issue: retained
exited-task fs_structs could keep overlay layers busy. Last fs membership retirement
now releases cwd/root use and traversal pins. Later pivot_root and accepted path
writes cannot reactivate that retired fs_struct. Exited task bodies relinquish
namespace/cached traversal ownership, while real accepted namespace operations,
nsfs descriptors, file/image uses and live CLONE_FS members retain their resources.
The overlay conflict check remains enforced. Regression tests verify live pivot
updates, retirement, late query writes, accepted OFD lifetime and remount recovery.

The complete Docker harness passes default multiworker nginx, 64 requests with
8 concurrent clients, named-volume contents, docker exec, daemon restart,
environment restart, SIGKILL/launchd recovery and login-agent bootstrap. Earlier
startup HTTP timeouts are retained as failed evidence; final embedded-driver and
installed-service verification are recorded separately below when completed.

## GPU boundary cost

The previous allocation fix already removed the coherent shadow byte scans.
Fresh worker samples still show page-publication/commit metadata in some setup
windows: 371 of 2,088 inclusive Face Detection worker stacks (17.8%). This is not
whole-process CPU time or the fraction of the complete workload's wall time.
Other sampled phases have little or no such activity.

Canonical non-executable imported data now publishes through its original shared
allocation epoch once per joined hardware epoch. Writable binding membership is
tracked on original mapping creation/retirement. This removes repeated per-page
publication/version processing without adding byte copies, instruction counters,
normal-write checks, global barriers, or a second permission model. File providers
and executable mappings retain their original page-level hooks. Existing software
exclusive reservations include the shared epoch. Native tests cover actual ARM
writes, protection changes, executable transitions, unmap/remap and final owner
release; common-memory tests cover alias visibility and reservation invalidation.

The [same-source boundary fixture](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/vulkan-mapping-boundaries.c)
keeps a fully initialized 256 MiB coherent generic allocation live. Each of 100
iterations writes one CPU word, changes an unrelated scratch page R then RW,
submits a GPU fill, waits on the original fence, reads device timestamps and
checks all 1,024 GPU output words plus the CPU input. Setup is outside loop timing.
Three sequential repeats use the same original MoltenVK archive, no compiler,
VM, profiler or test server. Stable Debian remains running and stable Docker is
stopped. Native and guest ABI/compiler differences remain; the scratch page is
16 KiB native and 4 KiB Linux. Thermal state is nominal and low-power mode is off.

| 100 iterations, median | Native Vulkan | Dev before | Fixed Dev |
| --- | ---: | ---: | ---: |
| Wall time | 32.139 ms | 260.552 ms | 38.179 ms |
| Original GPU timestamps | 0.810 ms | 0.710 ms | 0.737 ms |

Fixed wall time is 6.82x faster in this boundary-heavy probe and 1.19x native wall
cost. GPU device execution is effectively unchanged; this measures removal of
host metadata work, not faster shaders. All output checks pass. It does not imply
that an arbitrary game/browser/compute workload becomes 6.82x faster.

The unchanged full Geekbench Vulkan control scores 60,704 before and 62,719 in the
initial candidate, both about 65.1 s including calibration and upload. Face
Detection is 24,046 vs 23,997; the total difference is largely Particle Physics
variation. These single samples do not establish a reliable end-to-end gain.
The final pinned Dev 1.5.5 run scores [63,175](https://browser.geekbench.com/v6/compute/6971970)
in 65.10 s, with all eight workloads intact. Face Detection is 23,929; Particle
Physics is 198,792. This confirms completion without demonstrating a broad
steady-state shader speedup. No claim/access URLs are included in this record.
Deployment verification follows after final lifecycle acceptance.

## Local validation

The exact runtime source passes both complete workflow jobs locally: seven shell
steps each on macOS and native ARM64 Linux. The Mac product's twelve workflow
steps pass, including native input/clipboard and manager streaming checks, both
channels and packaging. Production bundles are additionally built with the
verified embedded MoltenVK driver; SDK CI explicitly uses external linkage.

The 26 Mac and 55 HVF hardware tests pass. Final common code checks for iPad ARM64
FFI. An unrelated global backing-revision read test initially raced parallel tests;
its timing assertions now run in a subprocess, preserving production semantics.
All failed diagnostic outputs remain in ignored build evidence, without adding
normal-path checks or repeating unchanged CPU/graphics acceptance.
