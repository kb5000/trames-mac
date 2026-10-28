# On-demand native Mac windows

The Mac product remains a terminal-first Linux userspace environment. An
unchanged Wayland client maps a toplevel, and the host creates its native window.
No remote desktop server or full Linux kernel is started. The terminal continues
when the last graphical client closes.

Build with `bash scripts/build-mac.sh`, keeping the generated `Trames.app` beside
`trames-mac-run`. The minimal agent bundle provides AppKit identity and the
existing ad-hoc Hypervisor entitlement. Launch through the CLI as usual. AppKit
and the Metal presenter initialize only on the first mapped window. Native
compute can initialize its own driver without a window. `--no-windows` disables the
Wayland service; console-only sessions also avoid graphics initialization with
the default automatic mode.

Inside an initialized Alpine environment:

```sh
apk add weston-terminal font-dejavu
weston-terminal &
```

`XDG_RUNTIME_DIR=/run/user/0`, `WAYLAND_DISPLAY=wayland-0`, and
`GDK_BACKEND=wayland` are supplied by the host. Installing software requires the
usual guest network access. Existing rootfs and transactional storage options
are unchanged.

## Ownership and interaction

Metal reads the client's retained, file-backed SHM mapping directly. HVF imports
those same bytes into its physical address space, including multiple guest pages
in one host granule. Existing region admission and physical host-borrow gates
remain responsible for CPU/native access. A hardware regression checks original
file pixels through eviction, page-in, and three successive VM lifetimes.

The presenter holds each snapshot until GPU reads finish. Positive frame
acknowledgements additionally require every drawable to have actually presented.
Cancellation and skipped display never report successful presentation. Two unit
tests exercise multi-drawable completion, cancellation, skipped presentation,
GPU failure, and exactly-once release without requiring a desktop or HVF.

The common compositor publishes coalesced snapshot readiness outside its scene
lock. The main thread blocks in CF/AppKit event processing; the Linux thread
retains the existing worker scheduler. No UI polling or resource-service thread
is added. The final window releases cached Metal setup, while the Linux session
and its standard streams remain alive.

The native view uses AppKit's layer-update path rather than its default drawing
cache. Geometry changes and Metal presentation share a Core Animation transaction;
command submission is scheduled before the drawable is presented, without waiting
for GPU completion on the main thread. This follows Apple's
[`presentsWithTransaction` contract](https://developer.apple.com/documentation/quartzcore/cametallayer/presentswithtransaction)
and [`wantsUpdateLayer` contract](https://developer.apple.com/documentation/appkit/nsview/wantsupdatelayer).
Unchanged title, size constraints and drawable geometry are not reapplied per frame.
The toplevel's external CSD shadow is clipped to its window geometry; macOS owns
the native outer shadow. Descendant surfaces retain outlying popup rectangles.

Native close sends `xdg_toplevel.close`. Cmd-W closes the focused graphical
client; Cmd-Q ends the session. Physical keyboard, modifiers, pointer buttons,
scroll, host resize, minimize/zoom and client-side move/resize use the common
Wayland input and shell paths. Move/resize requests require the active same-window
button serial. Outlying popup/shadow rectangles retain their own bounded child
windows and share their parent's guest input coordinates.

Keyboard focus also updates the containing toplevel's `activated` configure
state. Switching between windows deactivates the old one and activates the new
one; losing host focus removes activation. Moving focus within the same
subsurface/popup tree preserves activation. This lets original clients render
focused decorations and a solid blinking terminal cursor, with a hollow cursor
when inactive, without changing keyboard routing.

## Local evidence, 2026-10-27

- A clean snapshot at iPad `c33b1c82` / runtime `b0c44d54` passed all **81** steps
  of the local native ARM64 Linux runtime CI. This covers the shared mapping,
  snapshot subscription, and invalid-client error changes. Existing unrelated
  local edits were excluded from that snapshot.
- The subsequent runtime `ef184a40` adds authorized move/resize metadata. Its
  affected Wayland unit tests, strict Clippy, and all **5** original C-client
  scene/text/input tests passed. Unchanged CPU/Linux/network scopes reuse the
  full CI result above.
- The paired iPad `trames-ffi` check passed for `aarch64-apple-ios`.
- Mac workspace tests, strict Clippy, project boundary checks and the signed
  release bundle build passed. All **7** explicit signed HVF/pressure checks
  passed, including the new external file-mapping regression.
- All **6** Mac session checks passed after moving Linux off the AppKit thread:
  processes/threads, external commands, shebang execution, persistent write/read,
  and interactive interrupt/stop/foreground/resize/terminal restoration.
- The unchanged Linux C client rendered two real native windows and completed
  120 frames. Native zoom resized window 2 to 1920x928 while window 1 remained
  240x160. Native close produced the client's success line, removed both test
  windows, and reached `TERMINAL_SESSION_STILL_ALIVE` in the parent Linux shell.
- Original Alpine `weston-terminal` 14.0.2-r1 rendered its shell. Input wrote
  `windowok\n` into `/root/windowmarker`; a separate CLI process reopened the
  persistent store and read the same bytes. Ctrl-D exited the graphical shell
  and runner successfully.

The final desktop pass completed against Mac code `52734401` (the owner's
manual build differed only by temporary frame diagnostics, subsequently removed):

- The owner tested the reopened terminal directly and confirmed normal behavior.
  Native movement logs record the manual window drags; the compositor continued
  publishing and successfully presenting hundreds of subsequent scene revisions.
- Real key input created `/root/finalmarker` and `/root/keymarker`; a separate
  headless CLI reopened the same transactional store and verified both files.
  Command execution and persistence were checked independently of screen capture.
- `mac-windows.py` passed console-only startup in automatic and disabled modes,
  then **three two-window lifetimes**, each requiring 120 real frame callbacks.
  All six native windows closed, and the parent Linux shell accepted a new input
  line and exited successfully. AppKit initialized only once across the cycles.
- The final signed release, strict Clippy, formatting, project-boundary check,
  and all non-ignored workspace tests passed. Explicit hardware and Linux session
  results above are reused for unchanged HVF/memory/session production code.

A subsequent activation follow-up uses runtime `1f2d613f`:

- The original C client verifies each of two windows receives an inactive,
  active, then inactive configure sequence. All five native C-client tests,
  Wayland unit tests and strict Clippy passed.
- The paired iPad target check and final Mac workflow passed. The three-cycle
  real-display/terminal-survival test also passed with the new runtime pin.
- The owner confirmed the final focused cursor is solid and blinking, and
  becomes hollow after switching away. The original Weston's protocol trace
  confirms the corresponding `xdg_toplevel.configure` state array is sent.

Exact binary/fixture hashes and source-scope reuse are recorded in
[the machine-readable evidence](mac-windows.json). The repeatable display check is:

```sh
python3 tests/integration/mac-windows.py \
  --runner target/release/trames-mac-run \
  --rootfs /path/to/verified-Alpine.tar.gz \
  --client /path/to/linux-aarch64-wayland-client \
  --libraries /path/to/original-wayland-libraries.tar \
  --output target/window-acceptance
```

Compile `tests/fixtures/wayland-client.c` with original libwayland-client and
scanner-generated stable xdg-shell code. The library archive contains its
original distro dependencies at their Linux paths. Run on an unlocked Apple
Silicon desktop. The script requires three two-window lifetimes with real frame
callbacks, checks console-only startup in both modes, then sends terminal input
after all graphical clients have exited. It does not synthesize host UI events
or acknowledge frames on behalf of the native presenter.

## Remaining product gates

The Mac presenter currently exposes the common US physical-key layout. Host IME,
clipboard and drag-and-drop integration remain follow-up work. SHM software
clients are the accepted rendering scope; the HVF Vulkan/external GPU ownership
bridge and Xwayland product integration are separate gates. Native chrome/CSD
polish and app-specific compatibility require further acceptance. This checkpoint
does not claim Firefox readiness or a signed iPad release.

## Firefox investigation (2026-10-27)

The original Alpine 3.22.6 ARM64 Firefox 142.0-r0 package was tested in an
independent guest store with four CPUs, a 4 GiB guest budget, normal Firefox
settings, Wayland, and an offline HTML page. Desktop dependencies include the
original EGL/GLES libraries, fonts, a D-Bus session and machine ID. The page's
JavaScript title marker and green Canvas were observed in both diagnostic and
clean signed builds. The owner confirmed that the page button and input update,
but interaction is extremely slow. This is not full browser acceptance: input
latency, navigation and shutdown still require validation, and some subsequent
runs terminated with status 139. Later bounded runs with both clock behaviors
did not reproduce that exit, so its cause remains open.

A locked/fully occluded native window is intentionally not presented. Its
Wayland frame requests remain pending until real presentation is possible.
A small independent Wayland client reproduced that wait; after unlocking, the
same client completed two windows and ten actual frame acknowledgements each.
Remote desktop automation being available does not imply WindowServer marks a
Metal window visible.

Host sampling of a clean build found an avoidable resident-image scan in
`Memory::prepare_hardware`. The runtime now skips it when the fixed-buffer pin
table is empty, retaining checks for live mapped pins. In seven local synthetic
trials with 16,384 resident pages and 2,048 admissions, median admission time
fell from 37.068 ms to 0.205 ms. This measures that operation only; it is not a
Firefox startup or overall browser speedup claim.

Input and frame acknowledgements also now share the guest monotonic clock.
Forwarding host uptime directly is incompatible with GTK's reconstruction of
Wayland input timestamps from guest `CLOCK_MONOTONIC`. The shared server binds
its clock once per guest namespace and timestamps receipt before queueing.
Independent native peers retain their existing timestamp contract.

The memory suite, clock-domain regression, original native C Wayland peers,
shared process conformance, signed HVF/pressure tests, and Mac CLI/storage/PTY
checks passed locally. iPad FFI cross-compilation also passed. Unchanged CPU,
network and graphics acceptance results are reused. Raw browser logs, sampled
addresses, stacks and screenshots remain private under ignored build output;
no browser-specific diagnostic code is shipped.

### CPU baseline

Two Time Profiler recordings used the published runtime `6cf9481f` and Mac
`c77cb857`, with no guest instruction trace, syscall metrics, or experimental
IPC batching. The first recording covers approximately 18–34 seconds after
launch; the second covers approximately 213–234 seconds, after the page loaded,
and includes a native button-click attempt. The latter is a post-load workload
sample, not a completed click-latency measurement.

Time Profiler recorded only running threads at a 1 ms sampling interval. The
15.71 s and 20.70 s recordings contain 18,647 and 23,577 running-thread samples,
respectively. These sum to 18.647 and 23.577 sampled CPU seconds across all
process threads. Each sample is assigned to one category; percentages use that
whole-process CPU denominator, not elapsed time or end-to-end input latency.

| Disjoint CPU category | Early run | Post-load click attempt |
| --- | ---: | ---: |
| HVF permission changes (`hv_vm_protect`) | 52.87% | 64.05% |
| Other page mapping, table maintenance and view reconciliation | 19.27% | 17.65% |
| vCPU execution path, including entry/exit | 16.15% | 11.44% |
| Remaining host work | 11.71% | 6.86% |

The vCPU category cannot separate guest instructions from opaque HVF entry/exit
work. It must not be described as pure guest compute. Main-thread and Wayland
CPU together account for only 0.77% and 0.23% of the two totals; this does not
measure GPU utilization or exclude all possible presentation latency.

Within permission changes, moving Hypervisor.framework's mapped-range metadata
accounts for 36.91% and 47.30% of total process CPU. These are framework range
array updates, not copies of framebuffer pixels. Permission changes reached
through repeated physical-page export account for 42.54% and 44.40% of total
CPU. Reading the export path explains why: even retaining an already live
storage handle starts and ends a host byte-access gate, which revokes and
restores hardware access.

An independent three-second stack sample after the second recording contains
6,564 snapshots across four workers. Permission changes occupy 40.01% of worker
stack residency, other mapping work 15.74%, vCPU run 3.15%, and other waits
40.08%. Those figures include off-CPU lock/queue waits and have a different
denominator from the CPU table. They corroborate contention but must not be
added to, or substituted for, the CPU percentages.

The data ranks redundant export permission transitions first, page-table and
view-maintenance churn second, and repeated syscall metadata borrows after
those. A storage-handle-only export requires a separate concurrency/lifetime
review before implementation; no change to that unsafe boundary is included
in this baseline. Actual byte reads and writes still need the existing access
gates. Future comparisons must repeat the same CPU measurements and separately
measure successful input-to-update latency. No Firefox speedup is claimed by
this baseline. Raw Instruments traces and stacks remain in ignored local build
output; only this filtered summary is published.

## Native device transport checkpoint (2026-10-27)

The Mac HVF frontend now recognizes the opt-in Vulkan NativeCall profile and
returns through the same shared registry, Vulkan backend and task waits used by
iPad. Full five-word stub/capability/operation authorization remains in the
common runtime. Other HLT/BRK and execute-permission failures retain their guest
fault behavior. Pure libc acceleration remains disabled on hardware execution.
No complete Linux kernel or virtual GPU is involved.

The unchanged Linux AArch64 fixtures and guest ICD were reused from the verified
iPad/Linux build outputs. Native execution used the official pinned MoltenVK
1.4.2 archive (SHA-256 `562a15a29bc358446a56a4091c5f7e08f604184187c1d34f712148b61ef17276`)
and the host Apple GPU. All cases disabled networking and windows:

| Case | vCPUs | Native calls | Result |
| --- | ---: | ---: | --- |
| Disabled capability, nonexistent native library | 4 | 0 | Original loader reports incompatible driver; native library never loaded |
| Compute, explicit mapping synchronization | 1 | 181 | All rounds and offset/remap checks passed |
| Same compute with concurrent callers | 4 | 181 | Results and compilation peer progress passed |
| HOST_COHERENT mappings | 4 | 179 | CPU edits, GPU results and remap persistence passed |
| Descriptor update templates | 4 | 181 | Compute results passed |
| Headless rasterization | 4 | 128 | Independent barycentric pixel checks passed |

The five enabled cases total 850 authorized NativeCall attempts in this run.
Retries can change attempt counts; these are not fixed API-call counts. Each
device teardown reports 0 MB of
remaining driver allocation at the driver's reporting granularity. This is a
bounded lifecycle check, not an exhaustive leak/stability proof. No case created
the presenter run-loop source or a native window. Transfer counters still show
upload/download work: `direct_fragment_bytes` means direct checked guest-memory
fragments, not zero-copy guest-to-GPU aliases. Coherent shadow synchronization
and ordinary hardware host-borrow protection are unchanged. No metadata-only
export/unsafe region-guard redesign is included in this checkpoint.

The CLI parses host options before selecting its execution entry. An explicit
`--no-windows` uses the main thread directly; guest arguments containing that text
do not alter the host mode. Automatic mode retains the existing on-demand
presenter. All three two-window lifetimes, each requiring 120 real frame callbacks
per window, passed again and the parent shell accepted a new input afterward.
This exposed and fixed a common-kernel stdin wait bug: infinite ppoll/pselect/
epoll input could otherwise be misdiagnosed as deadlock after the last socket
closed. The fix queries accepted descriptor identities/armed watches and applies
to both iPad and Mac.

Local format, strict Clippy, workspace unit tests, boundary checks and signed
release build passed; all eight explicit hardware/pressure tests passed. The
shared Linux/VFS tests and 194 kernel conformance tests passed (three existing
conformance cases remain ignored). The iPad `aarch64-apple-ios` FFI check passed.
Unchanged CPU/complete-Linux acceptance scopes reuse their existing evidence.
No iPad CPU/device ABI or guest artifact changes are required. General shared
native device buffers, NPU/Core ML and on-screen Vulkan remain separate gates.

Reproduce using verified fixture outputs and original libraries:

```sh
python3 tests/integration/mac-native-devices.py \
  --runner target/release/trames-mac-run \
  --rootfs /path/to/verified-Alpine.tar.gz \
  --fixtures /path/to/vulkan-fixtures \
  --loader /path/to/linux/libvulkan.so.1 \
  --icd /path/to/libtrames_vulkan_icd.so \
  --library /path/to/macOS/libMoltenVK.dylib \
  --output target/native-device-acceptance
```

[Filtered input hashes and results](mac-native-devices.json) retain only
acceptance metadata. Raw native output, display transcripts and experimental
patches stay in ignored local output. Durations are development acceptance
observations, not comparative performance claims.


### Raw guest memory and persistent translations (2026-10-27)

The shared runtime now separates metadata retention, raw guest transfers and
explicit Rust byte borrowing. Mac tables persist per Linux address space on each
worker. Bounded mapping deltas replace unchanged-image scans, contents do not
retire translations, and actual host code writers publish their code range.
Canonical private data uses one VM generation notification per joined epoch;
shared providers and code keep their required publication hooks.

An independently rebuilt old `main` (Mac `9b843ae`, runtime `4e5f027`) received
only the same generic input/title timing instrumentation. Both variants ran the
original Alpine Firefox and the same isolated profile, offline JS/Canvas page,
four CPUs and 4 GiB logical budget. No browser code or settings were changed.

| Mouse release to JavaScript title acknowledgement | Old main | Updated |
| --- | ---: | ---: |
| Independent clicks | 3 | 6 |
| Median | 3,081 ms | 726 ms |
| Range | 1,652–15,060 ms | 326–868 ms |

This small sequential warm-profile comparison gives a 4.24x median improvement.
It excludes drawable presentation and automation round-trip time. It is not a
native-browser comparison, cold-start benchmark or tail-percentile estimate.
Updated key-to-title samples were 359, 508 and 643 ms. Second-page navigation and
clean runner shutdown were observed. Interaction is still too slow to claim
near-native Firefox performance.

Exploratory 15-second CPU recordings place permission changes plus other mapping
maintenance at 67.60% of old process CPU and 10.75% of updated process CPU. The
windows do not represent identical workload progress; this is evidence of a
changed bottleneck, not an overall throughput factor. Updated View preparation
accounts for 23.98%; filesystem checkpoint/VFS/resource preparation remain
visible host costs. Raw samples and addresses stay private in ignored output.
The filtered [measurement record](mac-firefox-memory.json) preserves categories,
input samples, revisions and interpretation limits.

Full shared-runtime and Mac workflow checks passed locally: boundaries, format,
strict clippy, workspace tests and release builds; the shared reclaim example
also passed. Signed acceptance passed 13 HVF tests plus one product pressure
test. Three Linux SMP/paging cases passed atomic counts, 64 MiB checksums,
fork/COW and 16 MiB working-set eviction/page-in. The iPad FFI target compiled,
and its interpreter kernel conformance passed 194 tests with three existing
ignores. A final cold execution-mode token fix passed memory regressions and
its affected Mac/iPad checks; unchanged full-CI results were reused.

Structural regressions require unchanged address-space switches to perform zero
new exports or cached-page inspections, a one-page protection change to inspect
one cached page, raw reads to coexist with hardware writes without a borrow
wait, code writes to execute without consumer rescans, and inactive caches to
release physical RAM. Cache/history overflow has an explicit conservative path.
`TRAMES_MAC_INPUT_TRACE` enables local native input/title timestamps only when
requested; logs contain key codes/titles and belong in private ignored output.
Normal runs leave this diagnostic disabled.


### Resource scope and serial/worker handoff (2026-10-27)

The next round removes namespace/disk work from anonymous inodes and scopes
linked resize persistence to its owning filesystem. Timer discovery indexes
live timer objects instead of walking unrelated FDs on every scheduling query.
Canonical direct data frames update content metadata without rebuilding mapping
indices. Most importantly, serial and worker CPU dispatch now share the session
registration domain: changing the runnable CPU count no longer discards a VM's
complete View through a code/alias-domain mismatch.

The same original Firefox, warm isolated profile, offline page, four CPUs and
4 GiB logical budget produced these sequential observations:

| Mouse release to JavaScript title acknowledgement | Median, six clicks |
| --- | ---: |
| Previous main (`dc508fa`, runtime `c47078b`) | 572 ms |
| Anonymous-inode/filesystem ownership separation | 507 ms |
| Plus direct-frame refresh and timer index | 430 ms |
| Plus shared serial/worker registration domain | 54 ms |

The final six samples were 270, 52, 66, 56, 50 and 53 ms, a 10.51x median
improvement over this round's baseline. Key-to-title samples were 32, 47 and
37 ms. Second-page title acknowledgement and clean shutdown also passed.
These are small warm-profile samples, not a native-browser comparison or a
cold-start/P95 claim. They exclude drawable presentation and automation latency.

Exploratory CPU samples after domain handoff place View preparation at 9.85%
(360 ms sampled CPU), versus 39.00% (3,815 ms) in the immediately preceding
variant. Full-frame preparation and tree insertion no longer dominate the hot
leaves. The recordings do not represent identical progress, so those totals are
bottleneck evidence, not an independent throughput factor. The filtered
[measurement record](mac-firefox-memory.json) retains all four stages.

Signed regression coverage alternates serial execution and a real worker
thread eight times while requiring one complete snapshot build and correct
canonical data. Shared-runtime local workflow checks passed in a clean product
copy, excluding the paused IPC experiment. Mac local workflow checks, 14 signed
HVF tests, the signed product pressure test and three Linux SMP/paging cases
passed. Memory passed 122 unit tests; VFS passed 318 with one existing ignore.
iPad FFI compiled; interpreter kernel conformance passed 194 with three existing
ignores. Only six sandbox-blocked loopback tests were retried with local socket
access; the other 188 passes were reused.

Window diagnostics exposed two GPU-complete frames waiting indefinitely for
screen callbacks. A newer scene can now supersede a capped queue: cancelled
frames never acknowledge presentation, and their source leases remain until
GPU reads finish. A lifecycle regression verifies delayed completion and late
screen callbacks release each source exactly once. The existing opt-in window
trace records GPU completion and screen callback state.

The initial visual acceptance attempt ran outside the active console user
session, where captured content could stay on an old frame or skip presentation.
The subsequent foreground check below closes this visual acceptance gate. The
four-stage CPU/input results still cannot prove displayed-frame latency. No
workaround treats an offscreen/skipped drawable as presented.

#### Foreground visual follow-up (2026-10-27)

The user returned to the Trames desktop, and the shipped Mac `1f0a6fc` with runtime
`be0280a` was tested without a rebuild. Visible pixels updated to `clicks: 6` and
`input: abc`; the input box also showed `abc`. The second offline page visibly
loaded, and browser Back restored the previous count and text. Metal reported
successful screen-presentation callbacks, with no GPU failure callbacks in the
observed sample. The previous persistent old-frame symptom was not reproduced
in this foreground check.

Six native release-to-JavaScript-title samples were 280, 118, 430, 64, 97 and
275 ms (median 196 ms); key samples were 186, 77 and 377 ms. Actual foreground
rendering and screenshot capture add workload, so the earlier 54 ms observation
must not be used as a screen-latency result or directly compared across these
different desktop/session states. This verifies visible interaction, navigation
and state preservation; it does not establish input-to-photon latency,
near-native performance, cold-start/P95 behavior or general Firefox release
readiness. The filtered measurement record includes the separate follow-up.
Only validation records changed; unchanged full-CI and hardware results were
reused.


### Online browsing and CJK fonts (2026-10-27)

The test window initially used explicit `--network off`, which prevented other
websites from loading. It was restarted with `--network outbound`, four CPUs,
a 4 GiB logical budget, a 24-hour session limit and per-event diagnostics off.
Original Firefox visibly loaded HTTPS Example Domain and Google, including
search results. No certificate-validation bypass or Firefox security setting
change was used. The CLI's normal network default was already `full`; no
network implementation change was needed for this configuration issue.

The original guest lacked CJK glyph coverage. Installing Alpine's signed
`font-noto-cjk` package and running `fc-cache -fv` made fontconfig enumerate the
CJK faces. After restarting Firefox, Japanese Google labels and result text
rendered as glyphs rather than missing-character boxes. The README now provides
repeatable online browsing and font-installation commands.

The owner still reports slow overall browsing. System stack samples from actual
online pages show substantial runtime metadata/mapping and code-publication
work. They include blocked threads and are not Time Profiler CPU percentages;
the attempted Time Profiler attachment did not complete. Raw stacks, addresses,
web page contents and diagnostic output remain private in ignored output.
These observations do not establish a new browser latency or speedup result.

A first-write-fault prototype passed 16 signed HVF tests, but remains a private
comparison and is not part of main. The selected architecture keeps the thin HVF
CPU/MMU executor and shared userspace Linux kernel. It removes redundant work
from canonical memory, while preserving mapping lifetime, permissions and
file/device semantics. Native GPU/NPU services remain independent of windowing.

### Private code publication (2026-10-27)

The former policy treated every admitted private RWX page as written at each
joined epoch. That advanced code generations and caused metadata reconstruction
and host instruction-cache publication even when guest code had not changed.
Private canonical code now uses the existing one-VM dirty notice, just as private
data does. Actual host writes publish their range at the producer; guest writes
use the architectural IC/DC protocol. Shared providers keep their persistence
hooks. This does not add write-fault tracking or change iPad interpreter writes.

Local validation passed the complete runtime and Mac workflow commands, 123
memory unit tests, 16 signed HVF tests and one signed product pressure test.
The added hardware tests cover 64 joined RWX rounds without metadata/PTE churn
and a guest code writer publishing four instruction replacements to another
vCPU with guest cache maintenance. Existing host code publication, permissions,
storage replacement, COW and reclaim checks remain covered. Real Linux SMP and
paging acceptance also passed in four-CPU and one-CPU configurations.

The real online Firefox check uses the same isolated profile, outbound network,
four CPUs, 4 GiB logical budget and installed Noto CJK fonts. Stack samples are
wall/blocked observations, not CPU percentages; live page content and scheduling
also vary. No end-to-end Firefox speedup percentage follows from the metadata
regression tests. Raw traces and page contents remain in ignored local output.

The exploratory samples observed `sys_icache_invalidate` at 1,178 collapsed
leaves before and 22 after; `update_parallel_snapshot` remained at 2,369 and
2,257 leaves respectively (including helper names containing that symbol).
The [filtered record](mac-firefox-memory.json) includes the method and limits.
This supports the direction of the code-publication change while identifying
backing/snapshot refresh as remaining work. The live workloads are not matched
and these counts are neither CPU percentages nor a Firefox speedup ratio.


### Bilibili crash: platform hypercall collision (2026-10-28)

The reported session exited through the guest Firefox crash path. Private logs
showed failed task creation, deliberate null-write assertions and finally Scudo's
unmap abort. The decisive reproducer was independent of Firefox: four threads
repeatedly mapping and unmapping pages began failing when the ordinary allocation
cursor reached `0xc1000000`. A fixed-address probe returned `0xffffffff` as if it
were a successful 64-bit pointer, although the corresponding Linux mmap had not
reached the kernel. Its subsequent write failed. The original probe exited 139;
the 800,000-iteration mapping stress exited 42.

The EL1 vector used `HVC #0` with Linux arguments still in x0. On the tested Mac,
the virtualization stack consumed values in the platform's `0xc1xxxxxx`
namespace, changed x0 and resumed EL1 without the exit expected by the runtime.
Using private `HVC #0x5452` fixes the transport. The C adapter validates the
immediate, and the TLBI helper remains on its separate immediate 1. This is a
Mac backend fix; it adds neither an extra trap nor ordinary-memory tracking.
The [backend contract](../architecture/execution-backends.md#el1-exit-transport-namespace)
links primary platform and developer sources.

The fixed-address probe and all 800,000 mapping iterations passed. Seventeen
signed HVF tests plus one signed product pressure test passed; the new hardware
regression checks all general registers with platform-number-shaped x0 values,
including high-bit aliases. The maintained original-musl fixture now covers
fixed mappings across that boundary and above 4 GiB, alongside discard and file
COW. Its old observer now keeps the private-footprint bound while checking
resident-mapping release with RSS: preserved file pages are not fully charged
to footprint, and RSS can count multiple aliases of one physical page. Neither
number alone proves the number of unique physical pages.

A separate 256-task capacity was also reached during page startup. The common
kernel now accepts a product task capacity, and the Mac selects 1,024; the iPad
and common default stay at 256. This does not allocate stacks or host threads
up front. Clone admission, RLIMIT_NPROC and proc reports share the configured
value. Regressions cover 299 shared-VM children at capacity 300, rejection of the
next task and actual guest getrlimit after the initial Linux context is moved.
Shared-kernel conformance passed 196 tests with three existing ignored tests;
iPad compilation, local workflow commands and real Linux SMP/paging acceptance
also passed. Unchanged memory/reclaim checks were reused. One unrelated loopback
network test initially observed EAGAIN; its isolated rerun passed, and remaining
workflow packages were completed separately.

With four CPUs, the same 4 GiB logical RAM budget, outbound networking and Noto
CJK, original Firefox visibly loaded, scrolled and refreshed the Bilibili
homepage. It remained live for at least 11 minutes without the former Scudo,
signal-exit, fork-failure or child-launch log signatures. Shader capability
warnings and one software-compositor mapping warning still appeared; continued
visible output was checked separately. This finite check does not establish
all-site, video-playback or long-term release readiness. The
[filtered record](mac-firefox-memory.json) retains only method, limits and
aggregate outcomes. Original traces and page contents remain private.

### Local mapping scope and backing ranges (2026-10-28)

The common runtime now distinguishes backing content invalidation from pause
dependencies. Nonvolatile VFS mappings with retained canonical storage can
commit local address-space changes while other address spaces execute. Same-mm
threads and unknown coupled providers retain their existing pause boundary;
nonlocal layout/EOF operations still join. Backing refresh uses a bounded range
journal and file-offset alias index, with full fallback for layout/source changes,
expired history and providers without range support. Ordinary shared bytes retain
hardware visibility and the existing raw-access/borrow contract.

A fixed static ARM64 Linux ELF exercises 400 mmap/mprotect/munmap rounds while
two peer processes run, then 128 one-byte writes to a 4 MiB mapped file. Three
alternating fresh-session pairs use the same ELF, four vCPUs, 4096 MiB logical
budget, no network I/O and no profiler. Other acceptance jobs had completed; the
existing desktop browser was left running. Timings are guest CLOCK_MONOTONIC
phase measurements, excluding startup and fork setup.

| Fixed work, median of three | Previous binary | Updated binary |
| --- | ---: | ---: |
| 400 local mapping rounds with two peers | 2229.70 ms | 14.32 ms |
| 128 one-byte mapped-file updates | 5.59 ms | 1.97 ms |

Each variant executes 1495 guest syscalls with the same reported resident-byte
charge. Updated runs record 1200 local checkpoint groups and 1200 address spaces:
one space per mapping checkpoint. Separate regressions verify that readonly-library
peers and active canonical shared-file writers continue during a blocked local
commit, while CLONE_VM threads and coupled custom providers stop. At 8, 256 and
1024 mapped file pages, a one-page write checks exactly three mappings, including
shared and detached-COW aliases; truncation still checks all aliases and faults
past EOF. An old whole-file write cannot widen subsequent small updates forever.

Local checks passed: the common runtime workspace; 198 interpreter kernel cases
with three ignored; seven concurrency cases under ThreadSanitizer with a rebuilt
ARM64 macOS standard library; Mac unit/static checks; 17 signed HVF cases and the
signed pressure case; all three SMP/paging configurations; and iPad core/distro/FFI
compilation. Unchanged passing cases were reused; no CPU implementation, guest
artifact or native allocator change requires a repeated unrelated CPU/allocator
campaign. The updated Firefox visibly loaded Bilibili and presented a keyboard
page-down. This finite smoke check does not measure browser input latency.

The timing ratios apply only to this deliberately amplified mapping workload.
They are not Firefox speedup ratios, native-browser comparisons, all-site/video
acceptance or long-term release readiness. Registers still take the full HVF
round trip; fork still has eager writable detachment. Those remain separate work.
The [filtered record](mac-mapping-scope.json) includes source/binary hashes,
individual timings, counters and limits. Raw logs, addresses and page contents
remain private.

The [fixture](../../tests/fixtures/mapping-scope.c) can also be built with a static
AArch64 musl toolchain (`musl-gcc -O2 -static -pthread`). Run its ELF with:

```sh
target/release/trames-mac-run --rootfs /path/to/verified-Alpine.tar.gz \
  --no-pty --metrics --cpus 4 --memory-mib 4096 \
  --exec-file /mapping-scope=/path/to/mapping-scope /mapping-scope
```

### Batched register context (2026-10-28)

The HVF adapter now transfers complete architectural state through a private
EL1-only context. This removes 126 individual GPR/SIMD framework calls from a
normal syscall boundary and preserves the existing single private HVC. The
common runtime remains at the mapping-scope checkpoint; iPad and guest artifacts
are unchanged. The context consumes one host granule per initialized vCPU and
one page of the existing adapter VA reservation, with no extra user-VA hole.

Three alternating fresh-session pairs use the identical static ARM64 Linux ELF,
four configured execution slots, a 4096 MiB logical budget, no profiler and no
network I/O. One guest process performs 4096 warmup getpid syscalls, then measures
300,000 additional calls using CLOCK_MONOTONIC. Startup is excluded. No other
acceptance jobs ran during measurement; the existing desktop browser stayed open.

| Fixed work, median of three | Previous adapter | Batched adapter |
| --- | ---: | ---: |
| 300,000 short syscalls | 478.18 ms | 361.85 ms |

This is a 24.3% reduction in elapsed time for this short-syscall workload. Every
run has the same checksum, 304103 total guest syscalls and 53248 charged resident
bytes. It is not a Firefox speedup ratio, a native-Linux comparison or a claim
that guest computation is faster. Additional resident-reload prototypes did not
reliably improve the batch result and are not part of the shipped source.

Local validation passed 21 signed HVF cases plus the signed pressure case. New
cases cover all GPR/SIMD/control state across alternating tasks/roots, EL0 timer
fallback, EL0 denial of private-context read/write/execute, and interrupted save
and restore. The latter also injects old-task x0/SIMD values during partial
restore. The existing platform-HVC collision, borrow, paging, code-cache,
allocation-reuse and teardown checks continue to pass.

The final product also passed four real Linux signal/altstack cases against
unchanged native-Linux oracle outputs, four one/four-CPU counter and privileged
register cases, six process/session/PTY cases, and all three SMP/paging
configurations. Mac workspace unit tests and Clippy passed. Unchanged shared
runtime/iPad acceptance was reused. After a normal browser quit/restart, Firefox
loaded Bilibili and visibly presented keyboard page-down; this is a finite smoke
check, not input-latency, video or long-term stability acceptance.

The [filtered record](mac-register-context.json) includes individual timings,
source/binary hashes and scope. The [fixture](../../tests/fixtures/syscall-context.c)
can be built with a static AArch64 musl toolchain and run through `--exec-file`
with `--no-pty --metrics --cpus 4 --memory-mib 4096`. Raw logs, process identifiers,
addresses and web contents remain private.
