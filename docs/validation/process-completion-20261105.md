# Source process exit and CPU/mm return — 2026-11-05

Runtime `c05a7e915a91344597efb34373fea8b427e013a0` moves ordinary final-thread, exit_group and default-fatal cleanup onto original source resources. Namespace-init victims use original groups. The existing completion queue returns full CPUs/metrics and collects private Rc projections while unrelated CPUs continue. A cold member sharing an executing mm uses only that mm byte checkpoint, without rebuilding mappings.

External guarded borrows retain physical mm custody independently of parent notification. Original mm identity prevents PID reuse from redirecting cleanup. A deterministic provider/borrower race verifies that late dirty bits survive publication; disabling the fix loses byte 9 and makes the test fail. No new service thread, syscall RPC, polling bus, unsafe block or ordinary guest barrier is added.

Apple and native ARM Linux pass 316 process, 56 kernel and 160 memory checks; VFS passes 507/506 respectively. Apple process coverage combines 315 full-suite cases and the separately added mm-retirement test. Strict affected checks and iPad target compilation pass. Actual HVF/interpreter execution passes 25 lifecycle cases, GPU/video teardown, 30 Go GC starts, eight isolated Docker rounds and real H.264/Vulkan sharing. No physical iPad run is claimed.

Seven counterbalanced pairs, 128 two-thread groups with ordinary/private epoll controls:

| CPUs | Previous Dev | Candidate | Elapsed change |
| --- | ---: | ---: | ---: |
| 1 | 14.908 ms | 15.060 ms | +1.0% |
| 4 | 30.300 ms | 30.107 ms | -0.6% |
| 12 | 30.086 ms | 30.067 ms | -0.1% |

This shows no material throughput improvement. At 12 CPUs, scheduling switches fall from 1092 to 840 and exit(93) cohort returns disappear, but structural clone/fork returns take their place; cohort rounds stay 256. Multicore vfork return means increase by about 8–15 µs. The next performance target is structural creation/exec, rather than interpreting fewer exit stops as a measured speedup.

The unchanged-ELF native ARM Linux reference median is 7.661 ms in the existing 6-CPU test instance. It is reused from prior validation and is not native macOS equivalence. Builds and the Linux instance were stopped for Mac measurements. The measured candidate precedes only revision-pin/source-comment changes; the final pinned package passed focused exit/vfork checks and is separately identified in the evidence.

Mac Dev 1.5.3 is installed with this exact runtime pin. Stable 1.5.3 and business environments remain untouched. Structural creation/exec, explicit tracing/non-ALLOW policy, excluded owner resources and legacy waiter adoption remain joined; owner-local Rc destruction remains asynchronous work on its existing owner. Robust-futex owner-death support is not added.

[Shared architecture](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/coordination-boundaries.md) · [Evidence and samples](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/process-completion-20261105.json)
