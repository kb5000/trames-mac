# GPU overhead attribution, 2026-11-13

The measured bottleneck in Geekbench's slow image workloads is CPU shadow-memory
synchronization. The working shared-mapping path can be close to an identical
native Vulkan control. This investigation uses installed Dev runtime `b73f42b`;
no GPU implementation changes are made during measurement. Stable Debian and
Docker environments are running during these diagnostic measurements, as
requested. They are not replacements for the previous isolated scored run.

## Same-API controlled experiment

The [original C fixture](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/vulkan-coherent-benchmark.c)
executes the same Vulkan calls against native MoltenVK and the installed Dev
Linux/glibc ICD. Each case uses a 16 MiB buffer, 100 queue submissions/fence waits,
one CPU input word, and a native GPU fill of the last 4 KiB. Every input/output
word is checked after the fence. Original query timestamps measure GPU work;
wall time also includes submission, waits, query retrieval and output checks.
Setup/teardown are outside loop timing. Three sequential repeats are measured.
Both binaries use the same original embedded MoltenVK archive and Apple M6 GPU.
The compiler VM is stopped before timing. Native/guest ABI, C compiler and library
differences remain; this is a same-source/same-API control, not identical binaries.

| Allocation policy | Native Vulkan wall | Dev Vulkan wall | Native GPU time | Dev GPU time |
| --- | ---: | ---: | ---: | ---: |
| 16 MiB, generic/aligned | 27.24 ms | 27.96 ms | 0.661 ms | 1.053 ms |
| 16 MiB + 4 KiB, generic | 16.54 ms | 2410.37 ms | 0.605 ms | 0.917 ms |
| Original required size, dedicated buffer | 14.88 ms | 35.58 ms | 0.614 ms | 0.970 ms |

Values are medians. Dedicated allocationSize is exactly the driver's buffer
memory requirement, as required by Vulkan. No invalid padded dedicated allocation
is used. The generic extra tail is a legal allocation-size variation.

An extra 4 KiB changes Dev wall time from 27.96 ms to 2,410.37 ms (86.2x), while
GPU work stays around 1 ms. The native control has no corresponding cliff. This
establishes a software mapping/synchronization penalty, independently of
Geekbench Metal-versus-Vulkan shader/API differences. The aligned generic Dev
case is 1.026x native wall time; this small, noisy control does not establish
near-native performance for every workload.

## Real Geekbench stack samples

An unchanged Geekbench Vulkan diagnostic run is sampled with macOS `sample` at
2 ms intervals: 8 seconds in Background Blur, 15 in Horizon Detection, and 15 in
Edge Detection. It is intentionally stopped after the third sample (185.56 s,
exit 143), before upload. It is not a completed or scored benchmark. There is
no concurrent compiler, CI, second benchmark or window capture.

| Phase | Worker samples | Upload synchronization | Readback synchronization | Combined share |
| --- | ---: | ---: | ---: | ---: |
| Background Blur | 3,005 | 845 | 1,447 | 76.3% |
| Edge Detection | 5,882 | 1,962 | 3,519 | 93.2% |
| Horizon Detection | 5,863 | 1,895 | 3,373 | 89.9% |

These are fractions of sampled stacks on the named execution worker during each
window, including its non-synchronization/blocked samples. They are not exclusive
whole-process CPU percentages or fractions of the entire benchmark's wall time.
Upload and readback are separate `coherent_sync_scope` subtrees, not recursive
sums of the same stack. Most samples descend into the scalar synchronization
loop itself. The guest input buffer comparison and CPU readback dominate these
sampled windows; native shader compilation/HVF are not the largest stack subtree
on this worker in those windows. Other threads and unobserved phases can still
contribute work.

## Mechanism and misleading counters

`backend.rs` operation 17 adopts the original native mapping only when the entire
native allocation owns complete host mapping granules and its pointer is aligned.
On this Mac the relevant granule is 16 KiB. A generic allocation with a 4 KiB tail
fails the complete-granule condition and uses `CoherentBytes`: guest memory,
native memory and a baseline shadow remain separate. Dedicated buffer imports
already have an owned, aligned allocation route.

`coherent::Scope` conservatively includes complete reachable resource bindings,
not only the few bytes written by the CPU or GPU. At submit/readback boundaries,
`coherent::synchronize` walks 4 KiB chunks, copies guest bytes into staging, and
compares/merges individual bytes against the baseline. Repeated submissions of
large, mostly unchanged resources repeatedly scan those full ranges. A new
coordinator is not involved. The context graph lease covers this CPU work too.

The earlier 187.7 GB upload-scanning counter is useful activity evidence, but
its 4.05 GB download-scanning counter undercounts asynchronous readback:
`Lease::invoke_with_images` returns a transfer delta; `Lease::poll` returns only
the completion result. Poll-side transfer deltas are not merged into the session
statistics. The controlled probe even reports zero aggregate download-scanned
bytes despite correct GPU output and substantial readback CPU work.

Likewise, the earlier `gpu_wait_us=195404564` is elapsed pending-call time.
`Pending::progress` adds it after `Lease::poll`, including CPU readback before
returning, compiler/event waits and scheduling delay. It is not 195 seconds of
pure GPU execution and must not be used to partition CPU/GPU wall time.
Original Vulkan timestamp queries in the controlled test avoid that ambiguity.
No corrected full-Geekbench device-time or download-byte total is claimed here.

## Recommended order

1. Make generic original native allocations adoptable more often: reserve an
   owned complete-granule native backing while retaining the guest's requested
   size/limits and native image/buffer aliasing. Import only verified owned,
   aligned pages. Never expose a neighbouring driver suballocation or falsely
   label a generic allocation dedicated. Audit actual allocation size, budget,
   commitment, lifetime and partial-map handling together.
2. Correct async transfer accounting and distinguish device timing, CPU
   synchronization and pending-call latency. Reuse existing call boundaries;
   avoid adding counters/checks to normal guest instructions or memory writes.
3. Keep a correct fallback for native mappings that cannot be adopted. Reduce
   its invalidation/dirty scope using existing original memory ownership where
   possible. Optimize byte loops only after eliminating avoidable whole-range
   scans; do not add global barriers, polling or a new coordinator/service thread.

These are design recommendations, not shipped GPU optimizations. The published
5,876 Vulkan score and 94,256 native Metal reference remain the last complete
scored pair. A future allocation-policy change needs native/guest alias, lifetime,
coherent readback, partial-map and iPad compatibility checks before rerunning.

Raw logs, sample stacks, three-repeat measurements, source/binary SHA256 values
and the private JSON evidence are under ignored `target/gpu-overhead-20261113`.


## Stable deployment recovery and independent lifecycle regression

Restoring the two existing autostart environments exposed a separate current-main
container startup regression. Docker starts, but runc init exits; startup logs
include a stopped-process error and `/proc/self/fd` stream `EBADF`. Subsequent
starts encounter overlay `EBUSY`. A zombie runc init has no open descriptors but
still exposes its old overlay in mountinfo. The complete causal chain remains
unresolved; do not call this a GPU failure or assume one lifetime fix solves it.

Runtime `dfe6ff9` fixes an independently reproduced occupancy defect: proc/query
metadata alone previously kept overlay upper/work layers exclusive. Occupancy
now consults the existing actual layer-use lease, retaining `EBUSY` while an
accepted open file or original namespace topology still owns those layers. The
new negative control fails before the fix and passes after it; remounting,
copy-up, sync, and old proc metadata readability are exercised together.

All 608 VFS/hostfs cases pass (one unchanged manual File Provider generator is
ignored), as do strict VFS/Mac checks, 22 Mac tests, formatting and iPad ARM64 FFI.
The kernel integration run has 486 passes, one host pause/resume failure and
three unchanged manual profiles ignored. Only the failed, unchanged pause case
is repeated after concurrent checks end; it passes. No passing tests are rerun
because of report edits. The metadata occupancy fix alone does not fix the
observed runc startup problem on current main.

To restore service, the stable application is returned to the preserved signed
formal 1.5.4 package, runtime `eb047dc`. Both original containers start, and the
existing web endpoint returns HTTP 200. Environment contents, container images,
application data, configuration and autostart choices are preserved. Dev remains
on `dfe6ff9` for continued diagnosis; both installations verify their signatures.
The GPU research above remains pinned to its measured `b73f42b` binary; Vulkan
code is unchanged by this independent metadata fix. The current-main startup
regression is an outstanding release blocker, not a resolved issue.
