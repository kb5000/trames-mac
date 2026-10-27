# On-demand native Mac windows

The Mac product remains a terminal-first Linux userspace environment. An
unchanged Wayland client maps a toplevel, and the host creates its native window.
No remote desktop server or full Linux kernel is started. The terminal continues
when the last graphical client closes.

Build with `bash scripts/build-mac.sh`, keeping the generated `Trames.app` beside
`trames-mac-run`. The minimal agent bundle provides AppKit identity and the
existing ad-hoc Hypervisor entitlement. Launch through the CLI as usual. AppKit
and Metal initialize only on the first mapped window. `--no-windows` disables the
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
