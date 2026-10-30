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


## Forkserver and typed DNS follow-up (2026-10-30)

Runtime `478e5992a73e867c478f1cf54601fb42e7cc4dab` fixes two independently
reproduced compatibility failures in the common runtime. Neither fix adds a
Firefox preference, a new coordinator thread, a global lock or a memory policy.

A blocked `recvmsg` can initially be attempted by the joined kernel, leaving
`-EAGAIN` in CPU register x0 while its pending call retains the original arguments.
When SCM_RIGHTS later arrives, the worker must defer descriptor reception to the
owner before consuming bytes. That checkpoint previously discarded the saved
entry ABI and dispatched the syscall using x0 as a new FD. Guest-side tracing
showed forkserver `recvmsg(3, ...)` fail with EBADF while the owner actually
received FD `-11`; the real FD 3 still closed successfully afterward. This was
an argument handoff bug, rather than an incorrectly shared fork descriptor table.

The checkpoint now restores x0–x5 and x8 from its saved call before returning to
the joined kernel. Existing retained-OFD and poll ownership remain unchanged;
the SVC is not re-executed. A real worker regression queues one byte plus an FD,
starts from the owner-blocked CPU state, verifies all original arguments and PC,
and confirms neither the byte nor rights were consumed before the handoff.
It fails on the old implementation with x0=-11 instead of 3 and passes after the
fix. In the diagnostic Firefox run before this correction, 14 FSC launch warnings
were recorded. A subsequent three-site diagnostic run created further content
processes without reproducing those warnings. This does not guarantee that every
future subprocess failure has the same cause.

The DNS endpoint previously rejected every RR type except A/AAAA with NOTIMP.
It also omitted a valid question from those error replies. Echoing the question
is required for ordinary query matching; see [RFC 5452, section 9.1](https://www.rfc-editor.org/rfc/rfc5452.html#section-9.1).
However, fixing that alone does not remove the delay: musl deliberately ignores
NOTIMP/refusal and waits for another usable reply. The static Linux resolver
probe reproduced about five seconds per HTTPS/TXT request.

Typed IN queries now use the host's configured resolver through `res_nsend`,
with a private resolver context per call on Apple/glibc and stateless `res_send`
on musl, on the existing two bounded DNS workers. A/AAAA retain getaddrinfo and
platform name-service behavior. Real record bytes and upstream negative results
are preserved; localhost and virtual .trames names remain local. Oversized wire
answers request TCP retry rather than splitting compressed resource records.
Stripping an incoming OPT trailer also clears its record count. Invalid or
unsupported classes retain a matchable error response. This does not implement
a separate encrypted DNS client or claim complete platform split-DNS semantics
beyond these system resolver interfaces.

| Static Linux resolver probe | Before | Fixed |
| --- | ---: | ---: |
| localhost HTTPS/type 65 | 5,000.881 ms, retry failure | 0.708 ms, NO_DATA |
| localhost TXT/type 16 | 4,999.838 ms, retry failure | 0.187 ms, NO_DATA |
| localhost A | 3.380 ms | 2.519 ms |

An additional public-domain probe returned the host's real HTTPS NODATA in
2.630 ms, a TXT answer in 222.865 ms, and an A answer in 4.825 ms. These are
individual observations, not DNS throughput or page-load speedup estimates.
Firefox may still log a failed typed lookup for a genuine NODATA response;
the relevant correction is avoiding the synthetic NOTIMP retry timeout.

Local validation passes 67 network cases and 215 common kernel cases on both
macOS and native ARM64 Linux (three existing kernel skips), strict affected
Clippy, 47 signed HVF hardware cases plus the Mac pressure case, the iPad ARM64
FFI check, and the signed Mac release build. A pre-existing Clippy-only test
Vec was changed to an array. Unchanged memory/Vulkan/guest graphics results from
the previous section are reused. Temporary socket interposers and Rust tracing
are excluded from the product and retained timing build.


### Coherent shadow transfers without per-page hardware revocation

The first two compatibility fixes did not make page loading consistently fast.
A separate two-second post-Bilibili sample found 665 of 1,066 samples on GPU
worker 0 inside Vulkan coherent synchronization. Many descended into
`HardwareStorage::begin_host_access`/`end_host_access`, `hv_vm_protect` and the
hypervisor's mapped-range vector maintenance. The AppKit thread was idle in
1,062 of its 1,066 samples. These are thread sample counts, not whole-process CPU
percentages or proof of an entire navigation's time distribution. The sample
preceded recorded navigation, during session setup. The subsequent Navigate-start
→ unique loopback launcher DOM marker took 70.631 seconds, including 7.310 seconds
inside the Navigate command; its initial session-creation wait is excluded.

Runtime `42eb0c9d56e9a0bee963f286ab67970c6b6e08f5` removes this borrowed-slice
path from ordinary coherent uploads/downloads. The existing shadow mapping was
already comparing guest/native/baseline bytes. It now uses bounded owned buffers
and the common `CpuMemory::read`/`write` operations, whose hardware Views use the
existing raw-region access model. It creates no Rust slice referring to live
guest hardware bytes and therefore needs no extra stage-two revocation for
these copies. The bounded staging costs up to two 4 KiB stack buffers; it does
not introduce a persistent allocation, permission policy or synchronization
thread. Presentation continues through its separate native image path.

Downloads write only runs changed by the GPU. CPU-owned input bytes remain
untouched, unchanged output does not dirty guest pages, and a failed write rolls
back the uncommitted baseline so a later synchronization can retry correctly.
The original checked chunk path remains available for write-only mappings;
ordinary read permission is not newly required. Guest Vulkan synchronization
responsibilities and volatile native accesses remain unchanged.

A frozen hardware-export View regression fails before the change because an
upload enters a hardware byte borrow. Afterward upload/download complete without
any such borrow, while preserving independent CPU input and GPU output. Separate
tests cover a failed second output run and write-only mappings. All 43 Vulkan
cases pass on macOS and native ARM64 Linux with strict Clippy; the iPad FFI check
and signed Mac build pass. The real MoltenVK resource oracle also verifies 256
GPU results, dynamic offsets, texel buffers, storage images, pipeline-cache
round trips, pool reuse and final budget release. Earlier unchanged kernel,
network and HVF acceptance results are reused.


### Ordered page-load observations after the coherent change

The [filtered record](mac-firefox-navigation.json) retains all warmups and trials
for both builds, the earlier failed mixed-site stress batch, binary hashes, DNS
probes and validation counts. Each site uses a new Firefox process with the same
previously visited isolated profile, one excluded warmup, three retained trials,
1920×792 CSS pixels, 12 CPU slots, host-managed memory and default graphics/
security/process settings. No build, sampler or verbose HTTP log runs during
retained timings. The earlier in-app measurements are retained as a reference.

| Page | Before coherent change, three samples (median), seconds | After, three samples (median), seconds | Earlier in-app median |
| --- | --- | --- | ---: |
| Wikipedia | 4.745 / 9.628 / 6.112 (**6.112**) | 3.847 / 2.606 / 5.425 (**3.847**) | 0.554 s |
| GitHub | 10.307 / 7.127 / 14.272 (**10.307**) | 3.990 / 8.753 / 3.990 (**3.990**) | 0.472 s |
| Bilibili | 16.375 / 29.635 / 47.623 (**29.635**) | 11.341 / 13.097 / 12.319 (**12.319**) | 0.598 s |

All nine requested target markers complete in each per-site batch. The 47.623 s
sample is preserved even though Firefox's busy renderer overran its nominal
30 s implicit wait. An earlier mixed-site stress batch still aborted on a
Bilibili target-element RPC timeout; it is not silently discarded or relabelled
as a measured page-load duration. Third-party WebGL/context-loss, script-timeout
and chunk-loading messages are observations, not proven explanations for every
stall. Some chunk-loading errors also remain after the coherent change.

These are click-command-start → DOM-marker measurements, not pixel presentation,
complete page/resource loading or interactive latency. Ordered small samples,
cache changes and dynamic origin/CDN responses preclude a stable whole-browser
speedup claim. The reduced medians are encouraging, while the substantial gap to
the earlier in-app reference and the remaining variation are explicit.

Between successive Bilibili trials, returning to the unique launcher and finding
its next link took 13.378 / 40.508 / 40.934 seconds before this change versus
8.439 / 15.946 / 4.843 seconds after. This combined setup includes browser command
and DOM-wait overhead and is separate from the target-page times above. A later
standalone loopback transition was 6.812 seconds after versus 70.631 before; this
is a separate diagnostic observation, not a fourth controlled page-load trial.

The post-change two-second sample contains no `hv_vm_protect` frames, while
Vulkan coherent synchronization still scans/computes over mapped buffers. The
sampling phases differ, so no aggregate CPU-percentage improvement is inferred.
Reducing unnecessary whole-mapping scans or importing eligible native GPU memory
remains a distinct next step; this change does not claim zero-copy GPU sharing.
The normal Firefox profile is restored after testing without Marionette.


## Native display control and sandboxed memory checkpoints

The shared runtime now removes two sources of unnecessary full-cohort stops:

- An active Wayland/XWM bridge used to return every CPU at each host control
  check. The existing owner now pumps its bounded resource adapter while workers
  retain their Views/translations; it also retains native I/O progress with all
  jobs parked. Real-worker coverage completes wl_display.sync in one cohort;
  the old active-peer case rebuilt eight cohorts for eight checks.
- Firefox content processes install seccomp filters. Their structural memory
  calls previously bypassed local mm checkpoints. The mm owner now evaluates
  the exact entry once and runs ALLOW in that domain. Other actions retain the
  full joined handler with a cached result bound to number, PC, six arguments
  and immutable filter identity. The sandbox remains enabled. A stale entry or
  changed policy cannot reuse an earlier decision; denial leaves memory intact.

These are common Mac/iPad scheduler changes. They add no resource-service thread,
ordinary memory barrier or function-wide safety lock. Same-mm peers still stop
for structural mapping changes. Filtered calls do not use the optional hardware
exclusive batching window, so each new entry receives its own decision.

Runtime progression is `42eb0c9d56e9` → display-only `76d61313c421` → combined
`60b14caf1ef4`. Both products pin the combined runtime. Mac --metrics exposes
cohort counts, preparation/dispatch/drain time, and pending syscall/native
occurrences returned at full joins. Several pending calls can share a stop;
these are not unique initiating causes or CPU-time percentages.

Two separate 90-second Wikipedia startup/idle diagnostics observed madvise in
19,925 versus 8,391 returned entries, with 61,588 versus 52,239 cohort rounds.
The earlier run used the display-only behavior plus the same profiling additions.
Its exact three changed runtime source hashes are retained in the JSON record.
Different startup/background work and local CI overlap in the earlier run
preclude a controlled CPU-speed claim. Real-worker tests, rather than those raw
frequency ratios, prove that allowed filtered mappings keep unrelated CPUs
running and evaluate BPF once. Temporarily disabling the new admission makes
both unrelated-CPU tests fail.

### Ordered mixed-site navigation after the coordinator changes

Each phase uses one fresh Firefox process/profile, 12 CPU slots, host-managed
memory, default graphics/security/cache/process preferences and a visible window.
Every round visits Wikipedia, GitHub, then Bilibili. One warmup per site is
excluded and three formal target-marker observations per site are retained.
This differs from the preceding per-site batches, which launched each site in
a separate process. All nine formal target markers complete in each phase.

A two-second sampler overlapped baseline Wikipedia trial 2 and display-only
GitHub trial 3. Their raw observations stay in the JSON, tagged as overlapping;
only the summary medians below exclude them. The combined phase has no concurrent
build, local CI or sampler. The medians count DOM-marker readiness from click
command start, rather than presentation, full resource loading or usable-page
latency.

| Site | Before both changes | Display-only intermediate | Combined change |
| --- | ---: | ---: | ---: |
| Wikipedia | 9.743 s (n=2) | 11.055 s (n=3) | 8.280 s (n=3) |
| GitHub | 18.246 s (n=3) | 15.490 s (n=2) | 19.777 s (n=3) |
| Bilibili | 14.610 s (n=3) | 19.242 s (n=3) | 13.951 s (n=3) |

The display-only intermediate is not hidden: it did not produce a general page
speedup. The combined result also remains mixed, with substantial site/CDN and
browser-work variation; it does not establish a stable whole-browser speedup.
Ordered small samples, fresh profiles and evolving HTTP/DNS/CDN caches remain
uncontrolled. Raw samples include the display-only 49.565-second Bilibili outlier
and the configured implicit wait being overrun by a busy renderer.

Firefox emitted one sandbox-broker EMFILE in the display-only run. A later host
observation found 110 FDs with soft maxfiles 256; the guest VFS separately has a
fixed MAX_FDS=256. This does not establish transient host exhaustion, guest
capacity or retained ownership as the cause. The final nine-load run did not
reproduce EMFILE, and the descriptor cap remains a separate capacity issue.
Third-party chunk/script loading and WebGL/context-loss warnings still occur.
No cache, sandbox, process or graphics preference was relaxed to obtain these
results. Native coherent GPU synchronization still scans mapped shadows;
unsupported FD/task operations and necessary same-mm/backing synchronization
retain their existing boundaries.

Local validation passes 223 common-kernel tests per OS (macOS and native ARM64
Linux; three existing skips), four native bridge checks per OS, strict affected
Clippy, the ARM64 iPad FFI compile and a signed Mac release build. Unchanged
network/memory/VFS/Vulkan and CPU-executor results from the preceding source-bound
runs are reused. English contracts and the complete filtered observations are
in [region memory](../../runtime/docs/architecture/region-memory.md#filtered-memory-calls)
and [navigation data](mac-firefox-navigation.json).

## Scoped coherent synchronization

The next measured Vulkan cost was coherent shadow synchronization, which used
to inspect every mapped allocation at each submit or successful host wait. The
shared backend now uses each submission's existing retained resource graph to
bound those comparisons to actual buffer/image binding spans. An unpublished
result ledger retires completed spans after successful readback and keeps
intersections with still-pending producers. Previously published fence results
therefore cause no repeated memory scan. This is common Mac/iPad code; the Mac
product only exposes the new scan counters in `--metrics`.

The [shared contract](../../runtime/docs/architecture/region-memory.md#vulkan-coherent-resource-intervals-2026-10-30)
describes map-after-submit, partial maps, aliases, failure retry and resource
lifetime coverage. Active resource spans still use byte comparisons and copies;
this does not yet map guest CPU pages directly into Metal. No extra guest-wide
lock, new service thread, graphics preference or sandbox relaxation is added.

### Fixed-work Firefox comparison

`benchmark-firefox-vulkan.py` serves a local WebGL1 fixture with 16 retained 1 MiB
unrelated GL buffers and one small dynamic triangle buffer. Each trial performs
64 batches of 8 draws, with buffer update and `gl.finish()` in every batch, then
reads one pixel. It uses the unchanged Firefox → Zink → Vulkan → MoltenVK path,
12 CPU slots, host-managed memory and a visible native window. Each process has
a separate fresh automation profile and one excluded warmup; three formal trials
follow. No builds or samplers overlap retained trials. A two-second sampler only
overlaps the excluded warmup in the second control run. Its stacks include the
old global coherent scanner, but no whole-process CPU percentage is inferred.
The source-controlled fixture is byte-identical to the measured fixture; its
SHA-256 and all 16 warmup/formal records are in the filtered JSON.

The order is old control A1, candidate B1, old control A2, candidate B2. Both
controls use runtime `60b14caf1ef4` plus only the same scan-metric instrumentation;
the exact control source hashes are retained. Guest drivers, graphics settings
and host native driver stay identical. The old signed application is preserved
for the recheck rather than rebuilt between observations.

| Median of 3 formal trials | Control A1 | Scoped B1 | Control A2 | Scoped B2 |
| --- | ---: | ---: | ---: | ---: |
| Fixture elapsed | 5.889 s | 1.282 s | 7.446 s | 3.667 s |
| Draw/update/finish sections | 4.803 s | 0.764 s | 6.588 s | 2.707 s |
| Click command to result observation | 6.453 s | 1.332 s | 7.877 s | 3.843 s |

All 16 pixel checks return `[64,128,191,255]`, GL error 0 and no context loss.
The `gpu_work_ms` JSON field is JavaScript wall time inside the batch sections,
**not GPU hardware execution time**. A1/B1 fixture elapsed improves 4.59× and
A2/B2 improves 2.03×. Absolute timings vary substantially across processes; the
recheck supports a real reduction in this fixed workload but not one universal
browser acceleration factor. Whole-session scan totals include startup and
unequal idle/background periods, so their byte ratios are not controlled speed
measurements. The page's privacy-masked `Generic Renderer` string is not used as
proof of a particular GPU; the independent native-driver oracle identifies it.

One ordered public-page observation after A1/B1 completes all three DOM markers:
Wikipedia 2.930 → 2.786 s, GitHub 8.670 → 7.454 s and Bilibili 10.989 → 5.588 s.
These single samples depend on evolving HTTP/DNS/CDN caches and page contents;
they are not stable page-load speedups or measurements of fully rendered content.
Existing third-party resource and unsupported-feature warnings remain. Full
browser responsiveness, native performance and general GL/Vulkan completeness
are not established by this change.

### Reproduce and validation

Use a disposable localhost Marionette profile as described by the existing
navigation benchmark. The normal launcher keeps automation disabled. With that
process running and its native window visible:

```sh
python3 scripts/benchmark-firefox-vulkan.py --port LOCAL_PORT --output fresh-results.jsonl
```

The controller changes no graphics, security, cache or process preference and
refuses to overwrite an existing result file. Repeat A/B with fresh processes,
the same driver artifacts and no concurrent builds. Retain the excluded warmup
and all formal trials. The existing real-driver `resources` example supplies the
independent synchronization oracle; its old global-scanning negative control
fails with 74,240 compared bytes instead of the required 256-byte bound span.
The candidate passes nonzero/partial maps, inaccessible unrelated pages, empty
submit and repeat fence observations, plus both 256-result GPU checks and the
post-recording descriptor update. Native allocation accounting releases to zero.

Local validation passes 45 Vulkan unit tests and 223 common-kernel tests per OS
(macOS and native ARM64 Linux, with 3 existing skips), strict affected Clippy,
the ARM64 iPad FFI compile and the signed Mac release build. Six macOS localhost
socket tests initially hit sandbox permission denial; only those six are rerun
outside the sandbox and pass. Unchanged CPU-executor and guest graphics artifact
results are reused. Complete filtered samples, source identities and validation
scope are appended to [the GPU record](mac-firefox-gpu.json).

## Shared resource memory and completion notifications

The next implementation removes coherent shadows for eligible mappings rather
than making their byte comparisons faster. One initialized, aligned host payload
is imported into Vulkan with VK_EXT_external_memory_host and into the common
guest memory model. New ICDs negotiate the optional capability; unsupported
imports and old hosts/ICDs retain the scoped shadow fallback. The guest still
receives a guest virtual address. Shared allocation/page owners retain its bytes
through partial maps, fork, unmap and native device teardown.

Source-built MoltenVK also publishes fence completion after releasing its native
fence mutex. Fence/timeline waits park owned jobs on the existing run queue and
return the CPU slot. Completion and finite deadlines resume them without a new
wait/service thread or a 1 ms guest sleep loop. The existing future-host-signal
submission and image-acquisition retry loops remain separate. Normal browser
preferences, security, graphics features and the Wayland proxy remain unchanged.
The matching static archive must include the host observer patch; the runtime
rejects stale archives at build time. The same source builds for the iPad SDK.

The initial shared mapping prototype exposed another cost: every unrelated
backing change checked the imported pages individually. Its Firefox process
recorded 730,871,959 page checks. Immutable imported data metadata now uses one
stable domain per mapping, while live bytes and code/monitor versions remain
uncached. Executable mappings retain conservative observations. A unit regression
bounds the data-page checks to zero after a byte change. The metadata-only run
recorded 145,336,098 page checks; these unequal whole-session totals diagnose the
mechanism, not a controlled CPU-time or speed ratio.

The final implementation also publishes external code versions at the existing
successful Vulkan observation boundaries. Both serial/worker decoder identities
and exclusive tokens notice device writes without shadow writeback. Ordinary
data loads/stores do not query that token or acquire a new barrier. This preserves
the original application's synchronization responsibilities. Details are in the
[common ownership contract](../../runtime/docs/architecture/region-memory.md#shared-gpu-resource-mappings-and-completion-waits-2026-10-30).

### Fixed-work results, including intermediate runs

The fixture, graphics artifacts and browser preferences match the preceding
scoped comparison. Every process has a fresh profile and one excluded warmup;
all formal observations are retained. The same newly built ICD is used throughout:
the old control declines the optional host capability and takes its compatible
fallback. The control is runtime b97ab40 plus the already-published Mac product.
All candidates use the same source-built native driver with only the additional
completion observer. No builds, local CI or sampler overlap the retained trials.
The longer control and both later candidates explicitly activate the visible native window.

| Median | Scoped control A1 | Initial shared B1 | Metadata fix C1 | Scoped control A2 | Publication D1 | Final E1 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Formal trials | 3 | 3 | 3 | 9 | 9 | 9 |
| Fixture elapsed | 3.018 s | 2.705 s | 3.594 s | 2.667 s | 1.169 s | 1.211 s |
| Draw/update/finish sections | 2.271 s | 1.970 s | 2.662 s | 2.076 s | 0.658 s | 0.728 s |
| Click command to result observation | 3.249 s | 4.134 s | 4.295 s | 3.745 s | 1.223 s | 1.269 s |

All 42 warmup/formal pixel checks pass with `[64,128,191,255]`, GL error 0 and
no context loss. Final E1 includes the global decoder-epoch correction: the
decoder may cache a page identity until its address-space epoch changes, so a
per-page external version alone is insufficient for executable GPU mappings.
Both identities now change; ordinary data-only mappings keep their epoch stable.
Compared with control A2, final E1 fixture elapsed improves 2.20× and
draw/update/finish time improves 2.85×. Earlier small batches remain mixed and
include regressions; the process order is not randomized. These results
support the fixed workload, not a universal browser or complex-page speedup.
`gpu_work_ms` is JavaScript wall time inside those sections, not a GPU timestamp.
Automation transport and unrelated browser work can delay result observation;
the first A1 formal host observation is 9.810 seconds and is not discarded.

The final Firefox process reports 14 shared maps covering 29,360,128 mapped bytes,
with upload/download/scanned bytes and coherent range scans all zero. Control A2
still compares 1,733,468,052 upload bytes and 1,717,787,580 download bytes. Those
are startup/workload/background totals with unequal process lifetimes; do not
interpret their ratio as a fixed-load CPU percentage or memory-bandwidth result.
Zero bridge transfer does not eliminate native GPU resource copies required by
rendering. The page's masked Generic Renderer string is not device identification.

### Public-page navigation on the final shared-memory build

The same signed final runner is measured separately from the fixed WebGL fixture,
using runtime `94e603cd479d` and Mac product `9b7dbd9bcb5b`. The unchanged
[navigation controller](../../scripts/benchmark-firefox-navigation.py) visits
Wikipedia, GitHub and Bilibili in that order each round. A fresh isolated profile
has one excluded first visit per site, followed by three formal rounds. All 12
attempts succeed without a content timeout or automation failure. The visible,
focused viewport is 1920×792 CSS pixels with devicePixelRatio 1. CPU count remains
12, memory host-managed, and graphics/security/cache/process defaults unchanged.
No build, CI, sampler or verbose network trace overlaps these measurements.

| Page | First visit, excluded | Three formal samples | Formal median |
| --- | ---: | --- | ---: |
| Wikipedia Firefox article | 5.758 s | 5.407 / 12.295 / 4.355 s | **5.407 s** |
| GitHub Firefox source | 6.174 s | 10.900 / 9.765 / 6.781 s | **9.765 s** |
| Bilibili home | 11.230 s | 12.293 / 6.053 / 8.310 s | **8.310 s** |

The metric is click-command start to the existing DOM marker, not complete page
resources, screen-present time or video startup. Wikipedia still ranges from
4.355 to 12.295 seconds, so this build does not establish consistently fast public
navigation. The fixed WebGL result cannot substitute for these site timings.
Caches and dynamic HTTP/CDN responses are uncontrolled; earlier single-site
observations are not matched before/after trials. The in-app browser is not
remeasured here. All first visits and formal samples are retained in the
`public_page_navigation` section of [the GPU record](mac-firefox-gpu.json).
The original non-automation Firefox profile is restored after testing.

### Acceptance and reproduction

The signed HVF acceptance runs the original guest fixtures, including computation
with one/four CPUs, coherent memory, descriptor templates, headless rendering,
resource lifetimes and timeline waits with one/four CPUs. All eight enabled cases
use shared mappings and report zero bridge transfers/scans; the disabled control
initializes no native GPU. Timeline coverage includes 64-bit counters, wait-any/all,
timeouts, cross-thread signals and inter-submit dependencies. The native resource
oracle separately checks nonzero partial maps, 64 exact device-copy results,
code-version publication and retained guest bytes after device teardown; the
budget releases after its last guest mapping. The legacy zero-hint oracle still
passes its scoped transfer and 256-result checks. Explicit quota acceptance and
rejection remain correct, with final release to zero.

Local checks pass 155 memory unit tests (one existing skip) plus six integration
cases, 45 Vulkan tests and 224 common-kernel tests (three existing skips) on both
macOS and native ARM64 Linux. Six macOS localhost tests retain their successful
unsandboxed results; only affected mapping/scheduler checks are repeated after
follow-ups. Strict affected/Mac lints, the ARM64 iPad FFI check using the new native
archive, both Apple driver SDK builds and the signed Mac release build pass.
CPU execution and unchanged Mesa/loader results are reused.

Rebuild the native archive with the existing trames-ipad `build-moltenvk.py`
recipe and guest ICD with `build-vulkan-guest.sh`. Keep their returned/source-bound
artifacts paired with the runtime. The opt-in extended native acceptance is:

```sh
python3 tests/integration/mac-native-devices.py --extended \
  --runner /path/to/signed/trames-mac-run --rootfs /path/to/alpine-rootfs.tar.gz \
  --fixtures /path/to/validated-fixtures --icd /path/to/new-guest-icd.so \
  --loader /path/to/guest/libvulkan.so.1 --library /path/to/dynamic/MoltenVK.dylib \
  --static-driver /path/to/embedded/libMoltenVK.a --output /path/to/local-results
```

`--static-driver` records the archive used to build the runner. A statically linked
runner ignores the dynamic-library override; that argument alone is not proof of
which driver executed. Run the unchanged Firefox fixture controller with
`--rounds 9` for the longer comparison. All warmup/formal samples, intermediate source
identities and driver/ICD hashes are in [the GPU record](mac-firefox-gpu.json).

## Resource-scoped runtime projections (2026-10-30)

Navigation-time diagnostics found repeated cold work in the coordinator:
filesystem namespace checkpoints, process projections, and Wayland authorization
captures. The shared runtime changes three mechanisms, for both Mac and iPad:

- File sync retains its original data sync, then checkpoints only the file's
  owning filesystem when linked. Memfd/unlinked files have no namespace entry;
  tmpfs and detached mounts do not flush the unrelated persistent root. Named
  persistent errors, synchronous durability and the global sync API remain.
- Wayland grants are cached in the actual shared FD membership under its existing
  object mutex. Owner and worker close/dup/replace retire inactive grants at the
  membership change, without another lock or service thread. Native connection
  changes invalidate independently of FD membership. Shared inode registration is
  still resolved live; a guessed FD or copied bytes cannot mint a grant.
- Proc observations carry the exact union of required payload bits through real
  alias/FD traversal. Reading maps, environ or auxv constructs only that payload.
  A descriptor link into another proc file no longer promotes the request to
  Full and rereads every process's arguments. Requested mutable argv stays live.

The first caching prototype retained closed shared-file allocations until the
next capture. It was rejected before integration. The final cache belongs to
membership itself; a regression checks an actual worker pipe close retires its
inactive grants, followed by an owner memfd close returning its charged pages.
Removing retirement from the final implementation makes that regression fail.
Only the final owned-cache variant is integrated; both prototype timings remain.

### Ordered public-page observations

All phases use the unchanged controller and content markers, 12 CPUs,
host-managed memory, fresh profiles and a visible 1920×792 CSS viewport. Each
phase excludes one first visit per site and retains three formal rounds in
Wikipedia/GitHub/Bilibili order. The old signed binary is preserved for A2.
Driver, guest ICD/Mesa/loader and graphics/security/cache/process preferences
match. No build, local CI or sampler overlaps these retained observations.

| Median, seconds | Old A1 | Prototype B1 | Field-set C1 | Final owned cache D1 | Old recheck A2 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Wikipedia | 5.407 | 5.423 | 4.939 | 4.374 | 4.266 |
| GitHub | 9.765 | 7.396 | 7.830 | 9.143 | 7.697 |
| Bilibili | 8.310 | 13.380 | 11.398 | 9.938 | 9.296 |

All 60 first-visit/formal target markers complete, with no content timeout or
automation failure. The results remain mixed across sites and processes; they
do not establish a general public-page speedup. These are click-command-start
to DOM-marker observations, not complete loading or screen-present latency.
All samples, source/binary identities and limitations are retained in the
`resource_projection_follow_up` section of [the GPU record](mac-firefox-gpu.json).

Separate five-second diagnostic visits expose the mechanisms without entering
the timing summaries. Initial Wikipedia/GitHub coordinator snapshots contain
131/1857 and 200/1777 inclusive Wayland-auth samples. The first cached prototype
has 3/1743 and 3/1636. Its Wikipedia proc refresh still has 429/1743 samples;
after exact dependency unions, the separate Wikipedia snapshot has 12/1566.
These are wall-stack observations in differing visits, not CPU percentages or
controlled speed ratios. Memory-view preparation and persistent-root checkpoint
serialization/durable commits remain material work; they are not removed here.

Final local validation passes 330 macOS and 331 native ARM64 Linux VFS tests
(one existing skip per OS), plus 224 common-kernel tests per OS (three existing
skips). Strict affected/Mac lints, the ARM64 iPad FFI compile with the matching
native archive and the signed Mac release build pass. Unchanged CPU execution,
GPU-memory ownership and guest Mesa/loader validation are reused. The normal
non-automation Firefox profile is restored after timing.

## Resident file metadata and disk sync scheduling (2026-10-30)

Resident, stable file data mappings now retain a metadata domain independent of
ordinary byte versions. Byte updates already reach canonical shared storage;
only EOF/source/layout changes require another mapping preparation. Executable
aliases retain their original content versions and changed-range history under
a distinct identity. Deferred, volatile and unknown providers keep their original
conservative path. No extra ordinary-access permission check, barrier or byte lock
is added. In 8/256/1024-page alias tests a resident data write needs zero page
checks; the same executable case visits the three touched aliases. Truncate,
regrowth, external SharedFile size changes, COW and code/monitor behavior remain.

Eligible in-epoch ordinary disk-file fsync/fdatasync now parks its caller on the
existing two-worker disk pool and returns the CPU execution slot. Owned host
backing/filesystem resources cross to disk work; the original OFD stays with the
coordinator. Data sync and durable manifest commit preserve their original errors
and completion semantics. A per-store manifest transaction mutex covers snapshot
construction through commit; no global file-byte lock or wait thread is added.
RAM files stay inline. Mapped/overlay/directory sync and namespace mutations still
use joined paths; submitted disk work drains before structural mutation.

### Public-page observations

The unchanged controller records click-command start to the existing site-specific
DOM content marker, with the same graphics/ICD/driver, 12 CPUs, host-managed memory,
fresh profile per process, visible focused 1920×792 CSS viewport and DPR 1. Each
complete phase retains one excluded first visit plus three formal rounds per site.
No build, CI or sampler overlaps retained timing. The initial old-binary phase
aborts after a content process exits with SIGSEGV; its five successful visits and
launcher failure remain in the JSON record and are not a complete comparison.
The old-binary recheck also aborts on a content-process SIGSEGV after ten
successful target visits, including first visits. Its last Bilibili formal trial
never starts. The table retains partial sample counts; neither old phase is a
complete three-site three-round comparison. The optimized phase completes all
12 target visits, including the three excluded first visits.

| Median, seconds | Old fresh recheck | Optimized |
| --- | ---: | ---: |
| Wikipedia | 3.912 (3 visits) | 7.017 (3 visits) |
| GitHub | 8.449 (3 visits) | 6.433 (3 visits) |
| Bilibili | 8.784 (2 visits) | 14.699 (3 visits) |

Public-page results remain mixed and network/content state is uncontrolled; these
small ordered observations do not establish a general browser speedup. Content
markers are not full loading, presentation, interactivity or video-playback timing.
The baseline crash is retained as an unresolved observation, not a claimed fix.

Separate five-second optimized navigation samples confirm DiskSyncJob executing
on existing trames-disk workers, including manifest checkpoints. Coordinator
checkpoint samples remain: 256/1759 for Wikipedia and 348/1746 for GitHub, from
joined sync and namespace operations. View preparation remains 214/1759 and
207/1746. These are inclusive wall-stack observations, not process CPU percentages
or matched-work speed ratios. The failed baseline diagnostic is not comparable.
The next durable-I/O boundary is joined mapped/directory sync and namespace
transactions, rather than adding more per-byte or per-page checks.

Local validation passes 155 memory unit tests (one existing skip) and six memory
integration cases per OS, 333 macOS/334 ARM64 Linux VFS tests (one skip each), and
225 common-kernel tests per OS (three skips each). The final scalar read-counter
change repeats only the 15 affected service tests and lint. Strict affected and
Mac-product lints, the ARM64 iPad FFI compile with the matching static archive,
and signed Mac release build pass. Unchanged CPU/GPU-memory, native archives and
guest Mesa/loader validation are reused. All raw samples, failures, source/binary
identities and diagnostic limits are under resident_metadata_disk_sync_follow_up
in [the GPU record](mac-firefox-gpu.json). The normal Firefox profile is restored
unchanged after the isolated test processes exit.
