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

Native close sends `xdg_toplevel.close`. Cmd-W closes the focused graphical
client; Cmd-Q ends the session. Physical keyboard, modifiers, pointer buttons,
scroll, host resize, minimize/zoom and client-side move/resize use the common
Wayland input and shell paths. Move/resize requests require the active same-window
button serial. Outlying popup/shadow rectangles retain their own bounded child
windows and share their parent's guest input coordinates.

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

The final desktop pass is still pending. The Mac locked during input testing.
An input fix keeps the root view available while hiding only its duplicate
pixel layer; this must be checked on the unlocked desktop, together with
client-side drag/resize and repeated window lifetimes. Do not interpret the
successful earlier visual checks as final acceptance of those later UI edits.

A repeatable real-display acceptance is provided for the remaining pass:

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
