# Docker recovery and native GPU publication, 2026-11-13

Final runtime `c0829591f8652e55c3edb074f593f680efbb76b9`; Mac 1.5.5. GPU measurements use
`86dfa2c`; the follow-up changes task retirement, FIFO wait classification and TCP admission.
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

The final complete Docker harness passes default multiworker nginx, 512 requests with
8 concurrent clients, named-volume contents, docker exec, daemon restart,
environment restart, SIGKILL/launchd recovery and login-agent bootstrap. Earlier
startup HTTP timeouts are retained as failed evidence. The final embedded-driver
run passes all checks; installed-service verification is recorded below.

Final shutdown acceptance found CLONE_THREAD incorrectly rebinding the shared
process wait event to the newest worker's visibility. A parent wait then reaped
that thread's identity while leaving the leader PID visible as a zombie.
Systemd's shutdown kill loop kept observing these already-waited processes.
Only a new process binds its wait-event reap target; threads retain the leader's
original target. Nonleader exec still publishes its actual replacement leader.
The new regression fails against the original code and passes with the fix,
including eight joined threads and wait with a retained leader body. The same
Docker fixture now stops normally in 0.978 s instead of about 60 s. The complete
interpreter conformance run passes 488 tests with three explicitly ignored.
Source exec now explicitly retires every replaced thread identity, postponing
old leader retirement until the replacement is published. The source regression
keeps both old leader and another peer body alive, including late CPU returns,
1/4 CPU execution, RAM/disk images and successful/failed exec modes.

The final full Docker run also exposed nginx workers in a dead nested PID
namespace. Namespace init completion now consumes only its own exited/adopted
children after all original victims finish, before publishing init's exit.
Children whose real parent is outside the namespace retain their zombie and wait
status. A negative control reproduces the leak; kernel regressions cover both
child-first/init-first exit and the outer parent's wait rights. No process-table
poller or coordinator cleanup is involved. All 75 kernel tests pass. The full
embedded-driver Docker lifecycle passes with stop times 0.994/0.967/0.972 s.

The prior FIFO signal test now checks actual hidden-FD cancellation/reuse and
EINTR rather than the removed coordinator-owned pending_open field.

## TCP churn acceptance follow-up

Installed-service stress beyond the earlier 64-request harness uncovered a
separate internal TCP handle ceiling. Detached closing/TIME_WAIT endpoints
correctly outlive their guest FDs, but were counted together with new listeners
and accepted endpoints against a fabric-wide limit of 256. nginx then reported
accept4 ENOBUFS even with only eight concurrent HTTP clients. This was not CPU
contention or missing readiness notification.

Protocol handle allocation now grows on demand. Listener creation, replacement
on accept, outgoing connection creation, dynamic wildcard address updates and
native-gateway admission no longer consult that aggregate ceiling. Live-FD
admission and unrelated gateway/network budgets are unchanged. TCP close states
retain their original protocol deadlines; they are not discarded to free slots.
No coordinator, polling bus or service thread is added, and admission no longer
scans every stack socket just to count global capacity.

The new real-packet regression fails on connection 125 with ENOBUFS against the
old code and passes 384 sequential connections with the fix, retaining over 256
protocol endpoints while no client FDs remain. A dual-stack listener regression
also exceeds the old handle ceiling and verifies real IPv4/IPv6 payloads and
namespace teardown. The real Docker harness now uses 512 requests / eight
clients rather than 64.

Uninstrumented 2,048-request testing then exposed two additional dependencies:
connection tracking stopped admitting at 512 retained records, and choosing a
fresh random ephemeral port on every connect reused tuples whose remote peer was
still in TIME_WAIT. Temporary packet diagnostics identify new SYNs matching those
old peer tuples; the corresponding nginx request is never logged. These diagnostics
are removed from production. The original close-state quota fix alone is therefore
retained as a failed Docker run rather than presented as complete acceptance.

Connection tracking now grows on demand, retaining NAT identity, reverse lookup,
zone separation and original close/expiry deletion. Tests verify 1,024 translated
flows per family, actual rewritten reply checksums, expiry and final index cleanup;
the original implementation fails this test at its ceiling. Closed guest FDs also
retain local TCP port ownership while the original protocol endpoint is closing.
Automatic port allocation uses one randomized namespace start and advances through
the existing ephemeral range, skipping occupied ports and wrapping normally. This
avoids early tuple reuse and the per-connect random-number call. Tests cover wrap,
occupied ports, FD retirement and eight-connection bursts with real payloads.

With these admission/lifetime fixes and no packet/syscall instrumentation,
two successive 1,024-request batches with eight clients pass 2,048/2,048, in
1.719 s and 1.474 s. This is a local Docker functional/stress check, not a native
network throughput comparison. Installed-service acceptance follows below.

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
Installed-service verification is recorded below.

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

## Installed release acceptance

Both installed Mac Stable and Dev are 1.5.5, runtime `c0829591f8652e55c3edb074f593f680efbb76b9`,
with the original verified embedded MoltenVK archive. Bundle signatures and both
CLI symlinks are verified. The original standalone Docker and Debian services
were stopped for installation and restored, preserving all environment settings,
stores, volumes and autostart choices; the default Alpine environment stays
stopped. Production stop times were 1.086 s for Docker and 1.755 s for Debian.

The original Debian business container is running and healthy, HTTP 18180 returns
200, and systemd reports zero failed units. The original Docker `web` container
serves its original port 51858; 512/512 requests with eight clients return HTTP
200 and the same body digest (P95 37.19 ms). This is functional service recovery,
not a native network performance comparison.

The final embedded-bundle acceptance additionally passes daemon restart, full
environment restart, SIGKILL/launchd restart, login-agent bootstrap and original
named-volume contents; all three normal fixture stops finish in 0.983–1.002 s.
The final local workflows pass 7/7 macOS runtime steps, 7/7 ARM64 Linux runtime
steps and 12/12 Mac product steps. Interpreter conformance passes 488 tests with
three ignored; the earlier concurrent-run seccomp timing failure passes isolated
and in the complete final run without production changes. iPad ARM64 FFI checks
pass; installed iPad applications are unchanged.

GPU measurements still use `86dfa2c`: the subsequent process and TCP fixes do
not change the measured GPU path. The package is a local unnotarized build;
old signed applications are retained for rollback. Disposable test services and
the Linux CI instance are stopped.
