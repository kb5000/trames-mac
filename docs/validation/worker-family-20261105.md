# Original process families (2026-11-05)

Runtime `464b8ed091feda2ee5231290920cc54898b86490` supplies shared original
parent/subreaper state, adoption, wait/SIGCHLD targeting and namespace victim
identities. Source getppid and subreaper prctl entries no longer end the cohort.
Final CPU/mm/native process cleanup remains an owned lifecycle boundary.

Both products pin this exact commit. Apple and native ARM Linux pass 302 process
cases (three ignored) and 52 kernel cases. Strict affected checks, project
boundaries and physical-iPad-target compilation pass. Real HVF/interpreter
programs cover filtering, parent death, fork/exec and namespace shutdown, with
250 namespace repetitions across the initial and final correction rounds.
Thirty normal-preemption Go GC starts and eight isolated twelve-CPU Docker
lifecycle rounds pass. Physical iPad execution is not newly claimed.

The focused four-thread/1,000-round control benchmark changes from 272.096 to
3.704 ms at four CPUs and 259.220 to 3.757 ms at twelve CPUs. Single-CPU results
remain essentially unchanged. The exact-ELF four-CPU native ARM Linux reference
is 0.962 ms on the six-vCPU Lima instance; this is not a bare-metal Darwin or
browser throughput comparison. The diagnostic has 6,858 to 4 cohorts and 8,004
to zero returned prctl entries; three startup/service requests remain.

The locally signed Dev 1.5.3 bundle is installed with this runtime. Its pin-only
rebuild passes another six real-program cases. Stable 1.5.3 retains runtime
`d9c4a811faf316a51b1a8f33f055247943a18855` and its original executable hash.
No stable environment, business container or Techo frontend is restarted or
activated by this development batch.

[Source-bound acceptance, all samples and installation hashes](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/worker-family-20261105.json)
· [Current common boundary contract](../../runtime/docs/architecture/coordination-boundaries.md#original-process-families-and-completion-notifications-2026-11-05).
