# Android ELF program-header alias regression — 2026-11-24

Runtime: `378b5158d8a4b515dc4461d8f258d252cb8145ec`.
Mac application source: `0262da5` plus that runtime pin; unrelated local Mac
and iPad work was excluded using a temporary clean build checkout. Both Mac
1.5.5 channels were built with the existing embedded MoltenVK configuration.

## Failure and fix

The existing Stable Android environment remained in boot animation for hours.
`trames-hal-service` repeatedly faulted in Android linker64
`soinfo::prelink_image()+340`, reading address `0x40010e40`. Audio policy never
registered, and system_server stalled in StartAudioService before watchdog
restarts. The on-disk HAL binary matched its original build.

This small PIE has several 16-KiB-aligned PT_LOAD segments whose file offsets
are in the same first 4-KiB page. Mapping their file-page prefixes also exposes
aliases of the ELF header table. The loader replaced AT_PHDR with the last
alias (`0x40008040`), so linker64 inferred the wrong load bias and dynamic
segment address. Keep the first complete, surviving header mapping instead:
AT_PHDR is `0x40000040`, and PT_DYNAMIC resolves to `0x40008e40`. Existing
invalidation for a later segment that overwrites the header mapping is retained;
all mmap-visible file prefixes remain intact.

## Validation

- New synthetic PIE regression failed before the fix with the observed AT_PHDR
  mismatch, then passed. All 9 trames-elf tests passed.
- An offline probe using the actual exported Android HAL and linker64 reproduced
  the old incorrect addresses and verified the fixed addresses and readable
  dynamic table (DT_NEEDED).
- `cargo check --offline --manifest-path runtime/Cargo.toml -p trames-elf
  --target aarch64-apple-ios` passed. The common fix was also pinned in the iPad
  repository; iPad applications were not rebuilt as part of this Android repair.
- Both Mac release channel builds and strict codesign verification passed.
  Dev and Stable were installed in the existing per-user Applications directory.
- The original Android environment was stopped normally and APFS-cloned before
  testing. No image replacement, userdata reset, or HAL/audio bypass was used.
- A Dev runner booted that existing environment to the launcher. The original
  crashing helper now exits 2 with its usage text when run without arguments.
  HAL, audioserver and system_server PIDs remained stable across follow-up checks;
  audio policy registered, SystemUI and Launcher3 ran, and a guest screencap
  visually confirmed the launcher.
- The Dev diagnostic runner was stopped normally. The installed Stable app then
  started Android through its ordinary background lifecycle. It reported
  `sys.boot_completed=1`, `init.svc.bootanim=stopped`, media.audio_policy found,
  and Launcher3 as the focused window. Its system_server PID 208, audio HAL PID
  65, audioserver PID 86, SystemUI PID 374 and Launcher3 PID 702 remained stable
  in follow-up observations; the crash buffer was empty.
- Opening `android.settings.SETTINGS` succeeded. SettingsHomepageActivity became
  the focused window, and a subsequent screencap visually confirmed the rendered
  settings page. Core service PIDs stayed unchanged and the crash buffer stayed
  empty.
- Existing Debian production runner PID 16028 was preserved, its port 18180
  remained listening, and a read-only HTTP check returned 200.

Local diagnostic logs and screenshots are under
`/private/tmp/android-boot-audit-20261124/`; build/install logs and the coherent
pre-test Android clone are under `target/android-elf-fix-20261124/` in the iPad
checkout. These local artifacts are not source dependencies. This was scoped
loader and Android boot validation, not a full Android compatibility or runtime
CI claim.
