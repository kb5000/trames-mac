# Exec image replacement — 2026-11-05

Runtime `a116d834318d59ea51f3de119fc7e20ed44c58d8` moves execve/execveat into the existing creator-mm checkpoint.
The original Rc owner still prepares and commits the image, but unrelated mms
continue. Actual peer CPUs/statistics return before de-thread cleanup, old queue
members are removed, and the replacement enters the live queue before running.
Preparation failures preserve the original image, Files and background work.
Detached old memory retains its dirty journal and original last-borrow readiness,
independently of the new image and numeric PID. No new service thread, polling
bus, unsafe block or ordinary guest-memory barrier is added.

Nonleader exec preserves process visibility until its new task identity is
published. Cgroup/parent/wait relationships, process age and returned peer CPU
accounting follow the survivor. Old retry/open/AIO state, deadlines and temporary
wait-mask restoration are cleared while preserving the current mask. Accepted
cgroup termination targets the original process inbox/group even after its
captured leader retires; it cannot follow a reused TID. Pure custody-policy errors
finish on untraced continuous workers before mm quiescence, and allowed/deferred
policy tokens are consumed once. Tracing and lifecycle policy effects retain
their explicit boundary.

Apple and ARM Linux each pass 320 process and 59 kernel checks. Full-feature VFS
passes 510/509 respectively; the unchanged 162 memory results are reused. Strict
affected checks and iPad target compilation pass. The 39 real acceptance cases
include HVF/interpreter lifecycle and exec, FD/script/sealed-file execution,
30 Go GC starts, eight isolated Docker rounds, GPU/video teardown and H.264/Vulkan
import. No physical iPad run is claimed. One Linux socket case failed in an
earlier concurrent-build run; the final full suite and ten focused repetitions
passed. Its cause was not established, and no test was weakened.

Two untimed warmups, then seven counterbalanced pairs; each row measures 32
replacements/attempts with a busy sibling. The following are 12-CPU medians:

| Workload | Previous Dev | Candidate | Elapsed change |
| --- | ---: | ---: | ---: |
| execve | 87.228 ms | 22.775 ms | -73.9% |
| execveat with proc executable open | 159.794 ms | 74.945 ms | -53.1% |
| Exec from a nonleader thread | 86.043 ms | 31.297 ms | -63.6% |
| Failed exec attempts | 72.904 ms | 5.011 ms | -93.1% |

The 12-CPU execve and nonleader cases fall from 33 full-cohort rounds to one;
all 32 exec returns disappear. The execveat case falls from 65 rounds to 33:
its 32 openat calls on /proc/self/exe still refresh dynamic proc state through
the joined path. Failed attempts retain their actual proc-open/control boundary.
Single-CPU results remain within 0.8% of baseline; that caller path is unchanged.
These are process-lifecycle results, not Firefox or general CPU speedups.

The same-source OS controls are retained for comparison (32 operations):

| Workload | Native macOS control | Native ARM Linux, local filesystem |
| --- | ---: | ---: |
| execve | 111.713 ms | 0.672 ms |
| execveat with proc executable open | Not available | 2.637 ms |
| Exec from a nonleader thread | 92.836 ms | 2.598 ms |
| Failed exec attempts | 2.050 ms | 2.165 ms |

Darwin uses Mach-O/system libc, Linux uses the same static ELF as Trames, and the
Linux reference runs in the local six-vCPU ARM CI instance. They are separate OS
controls; filesystem location and scheduler conditions matter. An earlier Linux
shared-mount reference is retained as diagnostic data and is not used above.

The prior 128-group fork/exit workload changes -5.1%, +1.6% and -1.6% at 1/4/12
CPUs in the paired run. A noisy eight-round vfork sample prompted a longer
128-round confirmation: plain vfork is 103.3→104.2 microseconds at four CPUs and
107.8→110.1 at twelve; the owner-operation variant is 110.2→112.5 and
112.7→114.9. Both sample sets are retained. No broad throughput regression is
claimed from short latency samples or different absolute host conditions.

Remaining boundaries include original-owner scoped construction, dynamic proc
opens, tracing/security lifecycle effects, namespace administration and actual
excluded backing/native controls. Stable 1.5.3 and business environments are not
updated. Raw samples, hashes, negative controls and check scope are in
[the shared validation evidence](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/exec-custody-20261105.json).

To reproduce the fixture, compile [worker-exec.c](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/worker-exec.c) with
`musl-gcc -O2 -static -pthread` on ARM Linux. Import the resulting ELF as an
executable and run `worker-exec MODE 32`, choosing plain, at, thread or failed; use the same binary
and warmup/order for both runtime versions. The Darwin conditional fixture can
be compiled with `clang -O2 -pthread`; its `at` mode is not a Darwin reference.
