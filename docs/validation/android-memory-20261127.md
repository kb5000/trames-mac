# Android memory lifecycle and PSI reclaim — 2026-11-27

Paired Runtime: `4fb6a45b94318e664e3a14167753465223ad44a3`.

Mac output binds the shared memory-pressure resource to the original native
pressure event source and reports actual page-out completions. Apple APIs stay
at the Mac boundary; common Runtime remains platform neutral. No additional
coordinator, service or timer thread is introduced.

The signed release snapshot was compared with the current product Rust sources,
Cargo manifests and lockfile. Original Android 13 ART/Zygote/Reaper and lmkd
acceptance passed. Ten consecutive bounded 384 MiB runs selected the cached
adj=900 target automatically, returned about 64 MiB and completed the allocator.
HostManaged is a host hint, not an additional guest quota; forced host pressure
and physical-device validation were not performed.

Final ART paired median workload time was 405.94 → 392.18 ms with PSI enabled.
Actual GC pause maxima and System.gc call latency are reported separately;
this does not establish GC acceleration. Apple/Linux resource libraries passed
2,364/2,374 cases, with five existing ignores each. The affected original gVisor
set passed 250 cases with six skips per executor in the documented modes.

The source-checked iOS Rust archive and strict platform checks passed. Product
pin and execution-boundary checks use the paired Runtime commit; existing
unrelated working changes are preserved.

Full source/binary hashes, reuse boundaries, failure evidence and limitations:
[shared validation record](../../runtime/docs/validation/android-memory-20261127.md).
