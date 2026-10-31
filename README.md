# Trames Mac

Apple Silicon hardware execution product for the shared
[Trames Linux userspace runtime](https://github.com/kb5000/trames-runtime).
ARM64 user instructions execute through Hypervisor.framework and a small EL1
exception adapter; no complete Linux kernel is booted.

Download a packaged build, run its `install.command`, then open Trames.app for
native environment management or run `trames` in a terminal. The terminal path
automatically creates a verified Alpine environment on first use; Debian 13 slim
is available through `trames env create NAME --distro debian`.

```sh
trames env create work
trames env start work
trames --manager
```

See [environments, installation, backup and release workflow](docs/guides/environments.md).
For source builds:

```sh
git clone --recurse-submodules https://github.com/kb5000/trames-mac.git
cd trames-mac
bash scripts/build-mac.sh
target/release/trames-mac-run --manager
```

`runtime/` pins an exact common-runtime commit. Run `git submodule update --init`
after switching revisions. The build script uses a local ad-hoc Hypervisor
signature; it does not require the iPad development certificate or a JIT entitlement.

The default interface is a terminal. With a rootfs or `--storage` and no command,
the product starts `/bin/sh -l`. Normal external commands, fork/exec, threads,
waits, pipes, signals and PTY job control use the same `trames-session` process
kernel as the iPad product. Host terminal input uses direct fd polling and POSIX
raw mode with automatic restoration. Ctrl-C/Ctrl-Z, `fg`, and window size changes
are forwarded through the shared guest terminal model.

Use `--mount GUEST=HOST` or `--mount-ro GUEST=HOST` for a session-local native
shared directory. Host tools and the running Linux session read/write the same
files. For an inactive private store, `--fs ls/cat/get/put` offers explicit
streaming access without starting a VM or presenter. See
[file sharing, limits and native performance comparisons](docs/guides/file-sharing.md).

`--storage DIR` selects the existing transactional disk VFS. `--rootfs` installs
Alpine only if that store is not initialized; subsequent launches can use just
`--storage DIR`. Omitting storage keeps the in-memory filesystem. `--network`
accepts `off`, `loopback`, `outbound`, or `full` (default). A missing guest DNS
configuration uses the managed host resolver; existing guest DNS settings are
preserved. Memory defaults to `--memory-mib auto`: allocations grow on demand
in the host VM pool, with no fixed product resident-byte quota. Guest memory
information reports host physical/free/available memory, and host pressure can
return live pages to the macOS pager. `--memory-mib N` selects an explicit MiB
hard cap when needed. Released physical pages return while execution stays alive.
The independent dense accessible-VMA descriptor limit remains 16 GiB per address
space; host-managed RAM is not yet unlimited sparse virtual address metadata.

Linux tasks now run on multiple hardware vCPUs through the common worker
scheduler. `--cpus N` selects the limit (default: available CPUs, capped at eight
and the HVF limit). Shared file mappings and native/VFS access use canonical
physical pages with scoped hardware exclusion.

Live dirty memory can be paged out and restored. macOS pressure notifications
trigger batched reclamation; `--resident-mib N` optionally sets a deterministic
hardware working-set target, independently of any explicit logical guest cap.
This does not cap all process RSS. See [hardware execution and paging](docs/architecture/execution-backends.md)
for ownership, temporary backing storage, remaining fork costs and repeatable
hardware acceptance.

Wayland applications open native Mac windows on demand. CPU-only console sessions
leave the presenter uninitialized. Closing the last window leaves the Linux
terminal session running. `--no-windows` runs the shared kernel directly without
the Wayland service, presenter run-loop source or separate session thread.
Build with `scripts/build-mac.sh` and keep its adjacent `Trames.app` bundle beside
the CLI executable. The bundle supplies macOS application identity and signing;
the CLI keeps the same PID and standard streams when entering it.

For example, inside the Alpine shell, install a normal Wayland software client:

```sh
apk add weston-terminal font-dejavu
weston-terminal &
```

The host supplies `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY`. Shared-memory pixels
are presented through Metal, with no remote desktop server or complete Linux VM.
Physical keyboard, pointer, scroll, native close/zoom/minimize, and client-side
move/resize requests use the common Wayland seat and shell. Host clipboard and
IME integration remain follow-up work. Native GPU texture presentation is implemented;
see [hardware Firefox rendering](docs/validation/mac-firefox-gpu.md).
See [native window ownership and validation](docs/validation/mac-windows.md).

For Firefox browsing, use a network-enabled guest session. `--network off`
explicitly disables Internet access and is intended for isolated tests:

```sh
target/release/trames-mac-run --storage /path/to/guest-store --network outbound --cpus 4 --memory-mib 4096
# Inside the guest shell:
apk add --no-cache firefox dbus font-dejavu font-noto-cjk
fc-cache -f
dbus-run-session -- firefox --no-remote https://example.com/
```

The [Alpine Noto CJK package](https://pkgs.alpinelinux.org/package/v3.22/community/aarch64/font-noto-cjk)
provides Chinese, Japanese and Korean glyphs. Refresh the font cache after
installation and restart Firefox. Browser certificate validation remains enabled.
Real-site interaction performance is still under investigation; the offline
input acknowledgements in the validation record are not a general browser
performance or release-readiness result.

Experimental native Vulkan compute and offscreen graphics are available with
`--vulkan`, independently of windows. Use `--no-windows --vulkan` for a compute
server. The guest needs the shared Trames ICD and normal Vulkan loader; the host
needs a compatible native Vulkan library (`TRAMES_VULKAN_LIBRARY` can select a
Mac MoltenVK dylib). These binaries are not automatically installed or bundled
by this checkpoint. The shared runtime still owns authorization, resources and
waits. Native on-screen Vulkan and Firefox WebRender have focused acceptance.
General device-buffer mapping and NPU/Core ML remain follow-up work. See the
[native-device checkpoint](docs/validation/mac-windows.md#native-device-transport-checkpoint-2026-10-27).

Hardware acceptance on a real Apple Silicon host:

```sh
# Build tests/fixtures/process-kernel.c with a native Linux AArch64 static-musl
# toolchain, then transfer that unchanged ELF to the Mac.
python3 tests/integration/mac-session.py --runner target/release/trames-mac-run \
  --rootfs /path/to/verified-Alpine.tar.gz --fixture /path/to/process-kernel \
  --output target/session-acceptance
```

This checks process COW/pipe/wait, pthread TLS/futex/atomics, stop/continue,
external commands, a shebang exec, persistent files, interactive interrupt/stop/
foreground, terminal resizing and host terminal restoration. Existing
`mac-signals.py` and `mac-execution.py` cover Linux signal contexts and repeated
live physical memory reclamation. Hosted Mac build CI does not claim availability
of nested Hypervisor execution. See `SOURCE.json` for extraction provenance.

Run `python3 scripts/check-hardware.py` for the explicit local HVF/pressure tests.
The [acceptance record](docs/validation/hvf-smp-paging-summary.json) includes the
full native Linux CI result and live-page integrity/RSS samples.

Linux counter/register compatibility and a controlled native/HVF CoreMark
comparison are documented in the
[compatibility and performance checkpoint](docs/validation/mac-compat-coremark.md).
The paired benchmark uses one Apple Clang with explicit shared CPU tuning and
checks both standard inputs, CRCs, thread counts and minimum runtime.
