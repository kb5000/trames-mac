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

The common commit is `f9d3caded5580b4f7929191335e71ce55724a2f0`. The guest
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
