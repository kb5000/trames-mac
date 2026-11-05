# Original vfork release and cold projection completion — 2026-11-05

Runtime `7b91b6dc664606ef4c046f218df7fe5001052073` publishes vfork release directly after the dying
CPU returns its View. A busy old owner and a live peer CPU cannot delay the
parent. Sole-runnable producer admission follows its original release object.
Opaque owner FD kinds no longer refuse non-last source exit: logical retirement
is direct and private Rc cleanup is asynchronous on its existing owner.

Original weak mm roots preserve actual backing writes through undispatched
owners and are allocated only for real exits. Original family dependencies
order nested namespace and host completion, including ancestor-dead cases.
Four negative controls prove these mechanisms; no new service thread, polling
bus, unsafe, ordinary guest barrier or source refusal gate is added.

Apple/native ARM Linux each pass 314 process checks (three ignored). Unchanged
validated kernel/memory/VFS results are reused. Strict affected Rust checks and
iPad target compilation pass. Final paired HVF/shared-interpreter binaries pass
25 lifecycle cases, native GPU/video and real H.264/Vulkan sharing, 30 Go GC
starts and eight isolated 12-CPU Docker rounds. No physical iPad run is claimed.

The stock previous Dev hits the fixture's two-second multicore vfork deadline.
Seven final samples (eight rounds each) give median return means:

| CPUs | Ordinary Files | Unique owner epoll control |
| --- | ---: | ---: |
| 1 | 2025.9 µs | 4028.7 µs |
| 4 | 124.2 µs | 97.0 µs |
| 12 | 121.0 µs | 96.8 µs |

Native ARM Linux reference: about 39/37 µs in the existing 6-CPU/8-GB test
instance. This is not native macOS syscall equivalence. One-CPU vfork remains
about 2/4 ms, essentially unchanged. No finite ratio is claimed for a failed
baseline. Ordinary 128-group thread churn remains slower:

| CPUs | Previous Dev | Final Dev | Elapsed change |
| --- | ---: | ---: | ---: |
| 1 | 15.558 ms | 15.983 ms | +2.7% |
| 4 | 29.126 ms | 31.220 ms | +7.2% |
| 12 | 29.806 ms | 31.618 ms | +6.1% |

Seven counterbalanced pairs exclude builds and the Linux instance. An extra
stop guard had no consistent gain and was removed; lazy roots only changed the
first candidate by about 1%. This is a targeted dependency/liveness fix, not a
general CPU or Firefox speedup. Final CPU/mm, last/group/default-fatal entry,
source namespace victims, robust futex and excluded operations remain open.

Dev is installed after acceptance. Stable 1.5.3, business services and Techo's
frontend stay unchanged.

[Shared architecture](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/coordination-boundaries.md) ·
[Full evidence and samples](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/source-completion-20261105.json)
