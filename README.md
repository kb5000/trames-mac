# Trames Mac

Apple Silicon hardware execution product for the shared
[Trames Linux userspace runtime](https://github.com/kb5000/trames-runtime).
ARM64 user instructions execute through Hypervisor.framework and a small EL1
exception adapter; no complete Linux kernel is booted.

```sh
git clone --recurse-submodules https://github.com/kb5000/trames-mac.git
cd trames-mac
bash scripts/build-mac.sh
target/release/trames-mac-run --rootfs /path/to/verified-Alpine.tar.gz --storage "$HOME/Library/Application Support/Trames/Linux"
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

`--storage DIR` selects the existing transactional disk VFS. `--rootfs` installs
Alpine only if that store is not initialized; subsequent launches can use just
`--storage DIR`. Omitting storage keeps the in-memory filesystem. `--network`
accepts `off`, `loopback`, `outbound`, or `full` (default). A missing guest DNS
configuration uses the managed host resolver; existing guest DNS settings are
preserved. `--memory-mib` defaults
to 4096; it caps guest memory without allocating a fixed RAM image. Released
physical pages are returned while hardware execution remains alive.

The current hardware backend schedules Linux tasks on one host execution thread.
Guest pthreads/TLS/futexes work; multiple simultaneous HVF vCPUs, physical export
of shared mapping providers, and live-anonymous host-pressure paging remain
separate gates. Wayland protocol services are shared, but a native Mac window
presenter is not connected yet. The intended display model creates windows on
demand for guest Wayland toplevels and keeps the terminal session alive after the
last display window closes.

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
