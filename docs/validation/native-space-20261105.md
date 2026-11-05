# Original native mm lifetime (2026-11-05)

Runtime `28c941b294e336c26d19cdbbf3add839583adb25` owns GPU/video/audio roots by
actual shared-mm task references. Fork isolates them; CLONE_VM shares them;
exec replaces its original reference. Source/owner exits retire the same task
once. Native cache hits bypass mm membership locking, and source calls retain
their existing context/stream leases.

Kernel lifetime maps and owner abandoned-GPU polling are removed. Actual native
completion/unlock notifications keep pending work alive and retire it without
blocking a driver callback or unrelated headless execution. Gated tests fail
when completion-owned release is disabled or the old polling is reintroduced.
Repeated notifications, contention and completion consumed by another native
caller are covered. No new service thread, ordinary guest-memory barrier,
native permission policy or unsafe declaration is added.

Apple and native ARM Linux pass 309 process cases (three ignored), 61 Vulkan,
15 shared-resource and one audio unit case. Strict checks and iPad-target compile
pass. Real native GPU/video controls and hardware H.264 export/fork/SCM_RIGHTS/
Vulkan plane-pixel comparisons pass. Thirty normal-preemption Go GC starts and
eight isolated twelve-CPU Docker cycles pass. Physical iPad and real microphone
execution are not newly claimed.

Ordinary throughput is essentially unchanged: four-CPU GPU control is
13.236/13.246 ms before/after (native macOS reference 12.689 ms); video metadata
is 4.012/3.984 ms. These are regression controls, not a GPU-driver/browser speedup.
The samples precede the final notification-only correction and are retained
with that limit in the evidence; final native device/H264 checks pass separately.

Dev 1.5.3 is installed with the new pin and independently passes six native
control cases. Stable 1.5.3 remains on `d9c4a811faf316a51b1a8f33f055247943a18855`
with its original executable hash. No stable/business environment is restarted,
and no Techo frontend is activated. Final last/group/default-fatal task entry,
CPU/Memory projections, robust futex and actual owner controls remain open.
Original non-notifying-driver idle and native audio/decoder stop behavior remains.

[All source-bound samples and acceptance](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/native-space-20261105.json)
· [Common boundary contract](../../runtime/docs/architecture/coordination-boundaries.md#original-native-address-space-lifetime-and-completion-retirement-2026-11-05).
