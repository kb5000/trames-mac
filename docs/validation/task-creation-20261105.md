# Task creation and live admission — 2026-11-05

Runtime `27b3eba9e6423345e1381f3318f5da08aeee19d8` removes the full-cohort return from all currently accepted
fork/clone/vfork combinations. The original owner still constructs the Rc task
and VFS graph under creator-mm custody; unrelated address spaces keep executing.
New children enter the live queue with the same resource capture as initial jobs.
Actual retired task/domain rows are removed, and existing CPU workers expand
participation on demand. This is scoped owner work, not fully source-thread
construction.

Birth shares the original family relationship transaction with namespace shutdown.
Current cgroup limits and membership commit together; accepted kill delivery
marks actual member identities before notification, including newborn binding.
Record-lock PID translation uses the live original namespace. TID copyout faults
follow Linux post-birth semantics. An external guarded memory borrower no longer
forces a creation-only full join: fork isolates the child while retaining the
parent snapshot and dirty journal. Child-first writable COW preparation preserves
parent physical storage/hardware mappings. Parked parents do not reserve busy
CPUs; warm continuations prefer an idle original executor without starving new
work. No new service thread, polling bus, unsafe block or ordinary-byte barrier
is added.

Apple and native ARM Linux each pass 318 process, 58 kernel and 162 memory
checks; full-feature VFS passes 510/509 respectively. Strict affected checks and
iPad target compilation pass. Real HVF/interpreter runs pass 25 lifecycle cases,
GPU/video teardown, 30 Go GC starts, eight isolated Docker rounds and H.264/Vulkan
import. No physical iPad run is claimed. Existing unchanged checks are reused.

Two untimed warmups per binary/CPU count, then seven counterbalanced pairs,
128 two-thread groups with ordinary files/private epoll controls:

| CPUs | Previous Dev | Candidate | Elapsed change |
| --- | ---: | ---: | ---: |
| 1 | 15.517 ms | 15.086 ms | -2.8% |
| 4 | 30.890 ms | 30.262 ms | -2.0% |
| 12 | 31.091 ms | 29.973 ms | -3.6% |

At 12 CPUs the same workload retains 838 context switches and 128 futex waits;
full-cohort rounds fall from 256 to one and clone(220) cohort returns from 255
to zero. It still performs 255 scoped mm checkpoints, so the construction and
communication costs have not disappeared. Plain vfork round-trip falls from
129.0 to 103.7 microseconds; the variant with an owner operation is 102.7 versus
107.2 microseconds. The unchanged ELF's earlier native ARM Linux reference is
reused and retained in the evidence; this is a process-lifecycle microbenchmark,
not a Firefox or general CPU score.

Separate opt-in HVF diagnostics located 381 unnecessary parent mapping retirements
in the first candidate; preparing child COW storage first removes all 381.
Diagnostics are not used for timing. Initial regressing and build-overlapped
trials were not used as acceptance results. Negative controls prove the retained
backing journal and parked-CPU regressions, rather than relying on code shape.

Exec, tracing, non-ALLOW policy, namespace administration and actual excluded
backing/native controls retain their documented boundaries. Stable 1.5.3 and
business environments are unchanged. Details and raw paired samples are in
[the shared validation evidence](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/task-creation-20261105.json).
