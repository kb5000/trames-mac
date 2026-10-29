# Firefox native GPU validation (2026-10-29)

Stock Alpine ARM64 Firefox 142.0-r0 now renders through hardware WebRender and
WebGL2 on the Mac's Apple M6. The path is Firefox → GLES/EGL → Mesa Zink → the
shared Vulkan NativeCall interface → source-built MoltenVK → Metal. Wayland
exports the original native texture to the on-demand Mac presenter. No complete
Linux kernel, virtual GPU, software Vulkan device or GL-version override is used.

The tested fresh profile selects hardware WebRender with default graphics
preferences. Content sandbox and the normal Wayland proxy remain enabled.
`nsIGfxInfo.getFeatures()` reports `compositor=webrender`, hardware compositing
available and WebRender available. The separate optional `wrCompositor` feature
remains blocklisted; it is not required for this EGL hardware WebRender path.
Marionette is used only in the isolated acceptance profile, over localhost. The
normal launcher does not enable automation or system-access testing.

## Correctness fixes

The common runtime contains two fixes shared with iPad:

* A resident file backing must retain its canonical shared allocation before
  considering a deferred page-source handle. Previously a rebuilt snapshot could
  describe a resident retained hardware mapping with an unloaded ConcurrentPage,
  causing dirty publication to panic. A real memfd regression reproduces the old
  failure across registry replacement and passes after the correction. Untouched
  file mappings remain deferred; no extra page reads or reservation are added.
* A newly adopted GPU compile has not yet subscribed to a completion signal.
  Resuming it from a VM checkpoint must give it a worker turn to subscribe/poll,
  rather than querying an absent signal. A two-task queue regression reproduces
  the old `armed GPU wait` assertion. The fix introduces no polling bus, service
  thread or guest-wide rendering lock.

Static Mac MoltenVK also needs IOKit and the SDK's original `clang_rt.osx`
availability helpers. The conditional build recipe now links them while keeping
the iOS device/simulator SDK and runtime selections intact.

The guest stack needs a separate Mesa 25.1.9 correction. Its PBO upload fragment
helper omitted `nir_store_output.src_type`; signed integer uploads generated a
floating-point fragment output for an integer attachment, which Metal rejected.
The patch preserves the sample's numeric type and the existing helper's
signed/unsigned conversion metadata. It changes no shader algorithm or Firefox
binary, and does not manufacture a Vulkan/GL capability. Original Gallium
packages remain available; this opt-in stack supplies paired rebuilt EGL and
Zink libraries. It is not a replacement for every installed Mesa driver.

The initial GPU lifecycle commit is `f9d3caded5580b4f7929191335e71ce55724a2f0`. The guest
[patch](https://github.com/kb5000/trames-ipad/blob/main/patches/mesa/0002-preserve-pbo-upload-output-type.patch),
[build recipe](https://github.com/kb5000/trames-ipad/blob/main/scripts/build-zink-gallium.py)
and [GLES pixel oracle](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/zink-pbo.c)
are maintained alongside the existing shared guest artifacts.

## Local acceptance

The [filtered record](mac-firefox-gpu.json) contains the complete graphics feature
report, artifact hashes and validation scope. Browser traces, profile contents,
shader dumps and host addresses stay in ignored local output.

| Check | Result |
| --- | --- |
| Firefox `glxtest --wayland` | Real Apple M6 via MoltenVK, GLES 3.0; original Mac driver only exposed GLES 2.0 |
| Independent PBO oracle on original Mesa | Reproduces the integer-attachment compilation failure, exit 139 |
| Same oracle on fixed Mesa | Exit 0; 128 exact signed/unsigned integer components; two invalid GLES cross-signedness combinations rejected |
| Default Firefox graphics selection | Hardware WebRender available and selected, without force-enable preferences |
| WebGL2 | Context created; exact orange readback `[255,128,0,255]` |
| Native presentation | Real GPU texture sources and completed display callbacks recorded; decoration/SHM sources remain distinguishable |
| Native pointer and keyboard | Click count reaches 1 and ASCII input reaches the DOM; Unicode IME is not validated here |
| Native window resize | Guest viewport changes from 1024×603 to 973×578; native GPU frames continue |
| Native close button | Firefox and the session exit normally with code 0 |
| Memory/VFS | 150 memory and 326 VFS unit tests pass; demand-paging tests included |
| Shared kernel | 211 conformance tests pass, 3 existing cases ignored; includes all 23 queue tests |
| Signed HVF | 47 hardware tests and one app-pressure test pass |
| iPad compatibility | `aarch64-apple-ios` FFI check passes with both common fixes |
| Lints | Mac workspace/all-targets strict Clippy passes. Common affected packages pass with the pre-existing `manual_range_contains` style warning in unchanged signal-frame code explicitly allowed. |

The final signed, installed Mac application also passes the normal launcher
without Marionette: the page reports the exact WebGL2 pixel, and native input
updates the text field and click count. Its default-profile startup logs
content/utility subprocess launch retries before successful page presentation.
Startup latency was not characterized in this functional run.

This is functional acceptance, not an FPS or native-performance claim. VA-API
video decoding still has no Linux render device and is not hardware-accelerated
by this work. VideoToolbox integration is separate. Mesa continues to warn about
unavailable logic-op, border-color, line-rasterization and depth-clip capabilities;
general Vulkan/OpenGL completeness is not claimed. Firefox privacy-sanitizes the
page's renderer string to `Generic Renderer`; its graphics probe and the native
driver identify the actual Apple GPU.

## Wikipedia allocation regression

The first installed build could exit 139 while loading Wikipedia. Its trace
rejects a 2,113,536-byte Vulkan allocation with only 147,030,016 bytes charged
against a 1 GiB session quota. An older, independent 128 MiB context ceiling
causes the premature OOM; WebRender reports `OutOfMemory` and attempts a device
reset or software fallback.

Runtime `c1c07347650cc976df8f15eb861f5ea2f6b9f409` removes that duplicate ceiling.
Host-managed sessions also remove the extra session byte quota and 64 MiB
native allocation ceiling. The original driver reports its actual heap and
allocation failures. Live native memory and coherent baselines retain shared
accounting and release on final drop. Explicit memory-capped sessions keep their
selected bounded policy. Protocol validation and resource ownership still apply.

The same native-driver regression fails before the correction and passes after:
three 64 MiB allocations succeed under a 512 MiB explicit session quota; the
128 MiB explicit quota rejects the third; three 96 MiB allocations succeed in
host-managed mode. Every case releases the accounted bytes to zero. The
accounting test additionally accepts more than 2 GiB without allocating RAM and
rejects arithmetic overflow. Run it with:

```sh
cargo run --release --locked -p trames-vulkan --example resources -- --budget-only
```

The corrected signed Mac build loads Wikipedia with hardware WebRender still
active. WebGL2 retains the exact orange pixel. The user also opened YouTube.
No native-allocation OOM or panic appears in that browsing run, which closes
normally with exit 0. All 38 Vulkan unit tests pass. Updated shared-kernel
conformance passes 212 cases, with 3 existing skips: 206 pass in the sandbox and
only the 6 local-TCP cases denied by the sandbox are rerun successfully outside
it. The updated iPad FFI check and affected-package Clippy also pass.

## Native window size follow-up

Applying the guest's maximize state can make AppKit enlarge a 1024×768 guest
window to a 1920×928 native content view. The old bridge suppressed delegate
resize callbacks while applying a snapshot and never reported the resulting
size. Firefox therefore kept drawing its original narrow layout inside a larger
window. The bridge now publishes the final logical content size after applying
native geometry and mode, with duplicate requests coalesced until acknowledged.
Retina drawable pixels remain distinct from Wayland logical coordinates.

The installed signed application confirms matching native and guest dimensions:
1920×928 maximized and 1024×768 restored. Native GPU presentation completes in
the same run; all three existing window frame-lifetime tests pass. The normal
launcher has no automation argument. macOS can initially report the window as
occluded and the existing presenter then defers drawing until it is visible;
that first-frame wait is not evidence of a native main-thread deadlock. The
optional window trace now records that condition and both sizes explicitly.

## Reproduce and run

Build the pinned Mac MoltenVK archive using the existing guest/native-source
recipes in `trames-ipad` (`prepare-vulkan.py --apple` and
`build-moltenvk.py --platform macos --arch arm64`). Set
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` for those Xcode builds.
Then build this product with the returned archive directory:

```sh
TRAMES_MOLTENVK_STATIC_DIR=/path/to/native-macos-driver bash scripts/build-mac.sh
```

Build the paired guest libraries in native Alpine ARM64, using the original
Mesa archive SHA-256
`412df33a1bb3c785ed698555a3972118a37c458e7accf6ae53f4bb87b3db454a`:

```sh
apk add build-base meson ninja bison flex python3 py3-mako py3-packaging py3-yaml \
  libdrm-dev expat-dev zlib-dev wayland-dev wayland-protocols elfutils-dev \
  vulkan-loader-dev vulkan-headers
python3 scripts/build-zink-gallium.py \
  --source /path/to/mesa-25.1.9.tar.xz --output /path/to/guest-graphics --jobs 6
```

The local acceptance used the same pinned source, patches, native Alpine GCC
14.2.0 and Meson options in an isolated Alpine build root inside the existing
ARM64 Linux build VM. That VM was stopped before Firefox validation; it is not
part of Trames execution. The recipe retains original source and patches next
to the produced libraries for corresponding-source distribution.

Use an existing Alpine store containing original Firefox, GTK/Mesa dependencies
and fonts. The normal launcher imports graphics files into a versioned guest
directory, checks real GLES 3/MoltenVK first and creates a separate default
profile. It leaves existing Firefox profiles and preferences alone:

```sh
python3 scripts/run-firefox-gpu.py \
  --storage /path/to/firefox-store --graphics-dir /path/to/guest-graphics \
  --icd /path/to/linux/libtrames_vulkan_icd.so \
  --loader /path/to/linux/libvulkan.so.1 --cpus 12 about:support
```

Use `--network loopback` for offline test pages. Headless native Vulkan compute
still uses `--no-windows --vulkan` independently of this browser launcher.

## Responsiveness follow-up (2026-10-30)

Hardware rendering alone did not remove coordinator stalls. In a two-second
sample of the slow live browser, 268 of 277 coordinator samples were in proc
refresh and 241 were reading mutable guest argv. These reads repeatedly entered
the HVF host-access/protection path while guest workers waited. These are
coordinator wall-time stack counts, not percentages of whole-process CPU time.
After removing unrelated argv reads, a separate sample during Firefox's real
process-info query found 78 of 142 coordinator samples rebuilding descriptor
snapshots, including path resolution and socket metadata.

Runtime `2c9ab5d75e7653239649ff5e362f2375605975c1` projects these payloads by actual
VFS demand. Maps and stat/status requests no longer scan argv or enumerate FDs;
cmdline, fd/fdinfo, network tables and file-nr keep live observations. Newly
published PIDs are re-probed, and proc FD magic links combine dependencies.
There is no timer cache, new global lock, service thread or extra memory policy.
The shared interpreter and iPad paths use the same change.

A separate recvmsg regression reproduces an invalid EMSGSIZE for a 2 MiB receive
capacity with only one byte queued. Receive capacity is now clipped to a bounded
transport chunk and may complete short. Send lengths remain intact; signed-iovec
overflow returns EINVAL before consuming input. This is a confirmed compatibility
bug; its connection to every Firefox subprocess launch warning is unproven.

The [filtered responsiveness record](mac-firefox-latency.json) retains the
measurements, methods and limits. All variants use the same offline page, original
hardware graphics stack, 12 CPU slots, host-managed memory and a visible 1920×928
window, with a fresh isolated profile per variant.

| Observation | Before | Argv projection | Argv + FD projection |
| --- | --- | --- | --- |
| Three sequential `ChromeUtils.requestProcInfo()` calls | Batch exceeds 35 s timeout | 1.810 / 1.849 / 1.927 s per call | 0.607 / 0.602 / 0.593 s per call |
| Native mouse release → emitted title update, median | 42.95 ms, 7 updates from 10 clicks | 46.84 ms, 8 updates | 32.70 ms, 10 updates |
| Offline page / WebGL2 | Count 10 / available | Count 10 / available | Count 10 / available |

This is one ordered run, not a stable benchmark distribution. Intermediate titles
can coalesce, so only emitted titles have matched timestamps. The measurement is
not screen-present latency or a native-host performance comparison. The final
run includes a 234 ms first response. Timing capture preceded the final proc-FD
alias guard; that guard has its own live-argv regression. No exact speedup is
inferred from the old batch timeout, which has no per-iteration progress.

The final signed build passes 214 common kernel conformance cases (three existing
skips), 327 VFS tests (one existing skip), the iPad ARM64 FFI check and affected
package Clippy with the existing signal-frame style allowance. Its normal
launcher restores the original browser profile, loads Wikipedia and follows a
native click to the English main page, without an automation port. Raw traces and
profile data remain local. Unchanged CPU/HVF and Vulkan acceptance results are
reused.

This removes two sources of unnecessary global work, but does not establish that
all page sluggishness is solved. Process-statistics queries still rebuild common
task metadata and cost about 0.6 s in this probe; content/utility subprocess launch
retries still appear during normal browsing. Those are the next targets before
claiming consistently fast navigation. Video decoding remains separate.

## Complex-page click loading (2026-10-30)

The [navigation record](mac-firefox-navigation.json) compares the same public
URLs and content markers against the in-app browser at 1920×792 CSS pixels.
Trames keeps its window visible, 12 CPU slots, host-managed memory, the original
hardware graphics stack and default graphics/security/process preferences. One
Firefox warmup per site is excluded. Both browsers have previously visited the
sites; HTTP/DNS/CDN caches and response contents remain uncontrolled.

The main metric is host elapsed time from click-command start to a content marker
entering the DOM: the Firefox article's first relevant paragraph, a GitHub source
directory link, or the fourth Bilibili video-title link. This is not pixel
presentation, complete subresource loading, video startup or interactive latency.
The in-app API does not expose Performance Timing, so browser-internal FCP/TTI
numbers cannot be compared. Different engines, user agents and automation paths
also prevent treating this as a pure guest CPU slowdown factor.

| Page | In-app samples, seconds (median) | Final Trames warm samples, seconds |
| --- | --- | --- |
| Wikipedia Firefox article | 5.412 / 0.554 / 0.539 (**0.554**) | 5.346 / no target content by the 30 s wait / 1.788 |
| GitHub Firefox source | 4.322 / 0.472 / 0.416 (**0.472**) | 9.084 / 8.556 / 16.743 (**9.084**) |
| Bilibili home | 5.354 / 0.598 / 0.571 (**0.598**) | 37.343; the next return-to-launcher automation wait stalls, and the third trial is not started |

Failures are retained, not dropped from a success-only average. The Wikipedia
failure still shows the launcher after its target click. The Bilibili setup stall
is not assigned a fabricated second page-load time. Busy Firefox renderer waits
can exceed the configured implicit timeout; the actual 37.343 s sample is kept.
An intermediate GitHub attempt also displayed PR_END_OF_FILE_ERROR. No TLS check,
certificate validation, content sandbox or site-isolation preference was disabled.
These results show substantial remaining latency and instability, rather than a
reliable whole-browser speedup. There are no confidence intervals or randomized
browser-order trials.

Earlier controller prototypes are excluded: one could accept an old launcher
document during navigation; another treated a transient empty script result as a
failure. The retained [controller](../../scripts/benchmark-firefox-navigation.py)
waits for a unique new launcher marker and uses browser-owned implicit element
waits instead of repeatedly creating script sandboxes. It records every sample,
warmup, content timeout and automation failure. With an isolated guest Firefox
profile containing `user_pref("marionette.port", 2828)` and started using
`firefox --no-remote --profile /path/to/test-profile --marionette about:blank` in
the existing graphics environment, run:

```sh
python3 scripts/benchmark-firefox-navigation.py --port 2828 \
  --warmups 1 --rounds 3 --timeout 30 --output target/navigation/results.jsonl
```

The normal product launcher continues without Marionette. The benchmark's HTTP
launcher and remote control connect only over loopback. Keep the Trames window
visible and the other browser on an idle page while each run is measured.

### Confirmed mechanism fixes

Runtime `6427992d672f91636ceb493f4dade3a8c7e072e3` removes two independently
reproduced sources of unnecessary work:

* Host-managed Vulkan still had a 16,384-object context ceiling after its byte
  ceilings were removed. Every Vulkan call additionally swept a Weak<Object>
  list, and object admission scanned it again. An owner-thread lifetime counter
  now performs constant-time accounting, including deleted IDs retained by
  commands/descriptors until final drop. Host-managed mode uses native driver
  capacity; explicitly bounded/iPad sessions retain their selected object policy.
  Ownership, driver destruction, protocol bounds and native errors are preserved.
* Built-in shared-anonymous pages had stable retained physical storage and a
  complete version/check contract but did not declare versioned preparation.
  An unrelated backing notification therefore rebuilt every page's frame.
  Declaring that existing contract preserves the original guards, pause domains
  and memory ordering. A real 64-page regression changes unrelated preparation
  from 64 frames to zero; writing one page still refreshes its current metadata
  and live data without changing its hardware mapping generation.

The native descriptor oracle fails on the old host-managed implementation at
batch four. The fixed implementation creates/releases 20,480 real MoltenVK sets;
the explicitly bounded case accepts 12,288 sets and rejects the next 4,096-set
batch as before. Run it from the runtime workspace with the existing native-driver
build configuration using `cargo run --release --locked -p trames-vulkan
--example resources -- --objects-only`.

Local validation passes 151 memory tests (one existing skip), 40 Vulkan tests,
214 kernel cases (three existing skips), 47 signed HVF hardware tests, the Mac
pressure case, iPad ARM64 FFI compilation, affected-package Clippy and the signed
Mac release build. Unchanged guest graphics/network acceptance is reused.
The previous Bilibili run's descriptor allocation failure and abort motivated
the object investigation, but its exact peak object count was not recorded. The
native regression proves the policy bug; it does not prove that every browser
failure has the same cause. Complex-page latency still needs work on remaining
snapshot/worker coordination and cross-site/network failures. No page-load speedup
percentage is inferred across the differently instrumented prototype runs.
