# Android graphics recovery with the embedded driver — 2026-11-17

Installing a Mac CPU/SDK CI bundle marked `TramesVulkanLinkage=external` on a
machine without its external Vulkan loader caused Android graphics startup to
fail. `vkCreateInstance` returned `VK_ERROR_INITIALIZATION_FAILED`; the composer
HAL repeatedly faulted and SurfaceFlinger restarted. The installed bundle was
an `-external-vulkan` CI artifact, rather than the normal embedded-driver build.

## Correction

Rebuild both channels with the existing verified static MoltenVK archive:

```sh
CARGO_INCREMENTAL=0 TRAMES_VULKAN_LINKAGE=embedded bash scripts/build-mac.sh
CARGO_INCREMENTAL=0 TRAMES_BUILD_CHANNEL=stable TRAMES_VULKAN_LINKAGE=embedded bash scripts/build-mac.sh
```

The default build already selects embedded linkage through
`scripts/configure-native-vulkan.py`. External linkage is an explicit CPU/SDK CI
configuration and is not the deployment artifact for GPU-enabled environments.
An unnotarized package can still contain the embedded driver; signature channel
and driver linkage are independent choices. Before local deployment, select the
bundle whose `TramesVulkanLinkage` is `embedded`, not the CI external artifact.

Both Mac applications were rebuilt, signed, packaged and installed with embedded
linkage. Their CLI remains version 1.5.5 and runtime
`c2c9fd7c1365db94ff9d4b1235953e9eeac9eaef`. Mac production source is `b808e50`;
this recovery changes the build selection, not Linux/CPU/IPC implementation.
The native archive SHA-256 is
`78ff59dfb286cd5baa17bbf5d1b917a15a80f1f6c6daba348c508bac8c9a5adc`.

## Validation

- An APFS copy of the stopped Android store reproduces the external-driver
  failure and boots with the embedded driver. The original store is preserved.
- On the isolated candidate, boot completes and the composer/SurfaceFlinger
  processes remain alive. Android Settings opens successfully; actual mouse
  navigation to the Apps page was visually verified in the native Waydroid window.
- The first isolated cold boot showed a SystemUI ANR. Restarting SystemUI restored
  native input and settings navigation. This is not claimed as a general ANR fix.
- The installed stable environment cold-boots to the launcher without that
  recovery step. Its actual screenshot shows the desktop with no ANR dialog.
- The installed composer and SurfaceFlinger keep PIDs 64 and 88 across two
  observations 238 seconds apart. The crash buffer is empty.
- SurfaceFlinger reports ANGLE, Vulkan 1.2.0 and MoltenVK on the Apple M6 GPU.
- Installed Dev and stable bundle signatures and runner hashes match their
  embedded-driver build artifacts. Both Info.plists report `embedded`.
- Stable Debian and Dev Debian execute ARM64 commands. Docker is active and the
  existing `web` container is running after the orderly upgrade/restart.

The prior runtime macOS/Linux CI and Mac product checks are reused because the
implementation is unchanged. Both linkage-specific release builds, signing,
packaging and the real Android graphics path were checked for this correction;
unrelated CPU, syscall and physical-iPad acceptance were not repeated. The iPad
apps already embed their driver and were not replaced in this recovery.

The isolated test environment is stopped. Raw diagnostics, screenshots and
hashes remain under `trames/target/android-graphics-repair-20261117`; the corrected
unnotarized packages are under `target/dist/android-graphics-repair-20261117` in
the Mac checkout. Guest data and environment configuration are retained.
