# Mac environments and distribution

Trames runs ARM64 Linux userspace on Apple Silicon using its shared userspace
kernel. The terminal is the normal execution interface; the optional native
manager creates environments and opens their terminals. Linux GUI applications
open Mac windows when needed. Closing the manager does not stop an independent
terminal session. No privileged daemon, Linux kernel image or host Docker is
required.

## Install and start

Unpack the distribution and run `install.command`. It verifies the application
signature, installs to `~/Applications/Trames.app`, and creates
`~/.local/bin/trames`. For zsh and bash the installer also persists that directory
in login and interactive shell configuration. Open a new terminal after installing;
the installer prints a source command for updating an already-open terminal.
Repeated installs do not duplicate PATH entries or profile source lines. Existing
profile settings are preserved. `TRAMES_MODIFY_PATH=0` opts out of profile edits.
Zsh's `ZDOTDIR` and the managed helper's `XDG_CONFIG_HOME` are respected. An existing unrelated `trames` executable is preserved. Upgrades
replace the application, not Linux environments. The installer also accepts an
explicit application path; `TRAMES_APPLICATIONS_DIR` and `TRAMES_BIN_DIR` select
alternate per-user destinations.

Open Trames.app in Finder, or run `trames --manager`, for the native environment
manager. It provides creation, existing-store import, terminal launch, stop,
settings, shared-folder selection, backup, restore, deletion and diagnostics.
Operations run in child processes; downloads and disk operations do not block
the application event loop. Settings changes require a stopped environment.

Running `trames` without arguments in a terminal creates a default Alpine
environment if necessary, then starts its login shell. Creation downloads the
pinned ARM64 minirootfs over HTTPS and verifies its SHA-256 before importing.
Network/download failures preserve existing environments and can be retried.
Verified archives are cached. Noninteractive no-argument application launch
opens the manager; use an explicit `env start` for scripts.

```sh
trames env create work                         # Alpine by default
trames env create debian-work --distro debian  # Debian 13 slim
trames env list                               # JSON, including settings/state
trames env start work
trames env start debian-work -- /bin/bash -lc 'uname -a'
trames env terminal work                      # Open macOS Terminal
trames env stop work                          # Stop, sync and release store
trames env help
```

Alpine currently pins 3.22.6 aarch64. Debian pins the official debuerreotype
Debian 13/trixie slim ARM64 OCI rootfs at revision
`e36653f9967231373aeba7cff2102fd004ac4f4a` (2026-10-27).
The [official image source](https://github.com/debuerreotype/docker-debian-artifacts/tree/e36653f9967231373aeba7cff2102fd004ac4f4a/trixie/slim)
contains its OCI manifest/layer digest; both distribution hashes are compiled
into the shared runtime. `--archive FILE` permits offline creation with those
same verified bytes, not arbitrary unverified tar extraction.

New environments replace the image builder's resolver configuration with the
existing host-managed DNS endpoint. Subsequent launches preserve user changes.
Alpine uses musl/apk; Debian uses its original glibc/apt/dpkg and merged-/usr
layout. Debian supports normal userspace programs and an optional systemd service
environment. Linux kernel modules and complete Linux ABI compatibility are
not provided.

## Stable and development installations

The two product channels install and run side by side:

| Channel | App / bundle identity | CLI | Default registry |
| --- | --- | --- | --- |
| Stable | `Trames.app` / `org.trames.Trames.mac` | `trames` | `~/Library/Application Support/Trames` |
| Development | `Trames Dev.app` / `org.trames.Trames.mac.dev` | `trames-dev` | `~/Library/Application Support/Trames Dev` |

Each registry has its own environments, global defaults, locks and launchers.
Existing installations are the stable channel and keep their current data path.
Updating/reinstalling dev neither replaces the stable app nor opens its stores;
same-named Linux environments can run simultaneously in both channels. Manager
windows and version output identify the selected channel. Both use the committed
application icon. `TRAMES_HOME` remains an explicit registry override, independent
of channel; use separate paths when overriding it for parallel testing.

Source builds default to **dev**, including the source runner's paired bundle.
Explicit `TRAMES_BUILD_CHANNEL=stable` builds the stable product; the channel is
compiled into the executable, not selected by its runtime environment. A normal
dev rebuild does not modify the stable bundle. Install the desired built app:

```sh
bash scripts/build-mac.sh
bash scripts/install-mac.sh 'target/release/Trames Dev.app'
# An explicit stable build/install:
TRAMES_BUILD_CHANNEL=stable bash scripts/build-mac.sh
bash scripts/install-mac.sh target/release/Trames.app
```

Both installers can add their command to the same `~/.local/bin`. Their managed
PATH helpers are independent (`path.sh` / `path-dev.sh` under the Trames config
directory), so custom install directories also remain usable together. Existing
shell settings and the other channel's CLI link are preserved.

To test a copy of stable guest data, make an offline backup and restore it in dev:

```sh
trames env stop work
trames env backup work /tmp/work.trames.tar.gz
trames-dev env restore work /tmp/work.trames.tar.gz
trames-dev env start work
```

Restoring creates a separate store and machine identity; shared host folders need
explicit re-selection. It does not turn a live stable store into a dev store.

## CLI configuration and default commands

```sh
trames config show
trames config path
trames config set default-environment work
trames config set cpus 4 memory-mib auto windows off
trames config set distro debian init systemd
trames config reset
trames env config work
trames env configure work -- /bin/bash -l
trames env start work                         # Saved command
trames env start work -- /bin/sh -ec 'uname -a' # One-session override
```

Global settings are saved atomically to `TRAMES_HOME/cli.json` (the usual registry
under Application Support when `TRAMES_HOME` is unset). `default-environment`
selects the environment used by no-argument terminal launches. If missing, it is
created with the configured defaults. The `defaults` object affects **new**
environments; it does not rewrite existing environment settings. Creation flags
override those defaults, and explicit start arguments override saved session
settings. `env config NAME` prints the saved per-environment JSON.

Global `config set` accepts `default-environment`, `distro` (`alpine`/`debian`),
`cpus`, `memory-mib`, `network`, `init`, `windows`, `vulkan`, `video`, and `command`.
The last is a JSON argument array, for example
`trames config set command '["/bin/bash","-l"]'`. Per-environment commands can
instead use the simpler `env configure NAME -- COMMAND ARGS...` syntax. Arguments
are stored literally and never evaluated by a host shell. Existing environment
configs without `command` continue to use `/bin/sh -l`. `config reset` resets only
global defaults. Systemd boot retains its original PID 1 command independently of
the saved shell command; an explicit `--init shell` enables maintenance sessions.

## Custom ARM64 Linux rootfs

```sh
trames env create custom --rootfs /path/to/rootfs.tar.gz
trames env create custom --rootfs /path/to/rootfs.tar --sha256 EXPECTED_SHA256
trames env create custom --rootfs /path/to/unpacked-rootfs -- /bin/bash -l
trames config set default-environment custom
trames env start custom
```

`--rootfs` explicitly selects a user-supplied rootfs directory, ordinary tar or
gzip tar, independently of the built-in image catalog. The manager's Create
dialog also offers a custom-rootfs picker. The image must contain ARM64 Linux
userspace and the chosen command/interpreter (default `/bin/sh`). The entrypoint
is checked before publication; x86 and macOS Mach-O programs are rejected. Other
binaries, drivers, system calls and services retain the shared runtime's actual
compatibility limits. A successful import is not complete distribution support.

Custom imports snapshot the source, hash it and stream file contents into the
private VFS. Compressed data and expanded tar data use temporary files, without
whole-rootfs RAM buffering. File permissions, ownership, modification times,
symlinks (including merged-/usr layouts), hard links, FIFOs and Linux device-node
metadata are restored. Directory symlinks are copied as guest links, never
followed into host paths. No host device or mount operation is performed. Host
source directories must be an inactive rootfs, with transient kernel filesystems
unmounted. Extended attributes/ACLs and sparse-tar formats are not imported yet.
The existing store byte/inode capacities still apply.

`--sha256` verifies the supplied archive bytes; the actual source digest is
recorded in the environment config. For directory inputs it records the generated
tar snapshot's digest; `--sha256` is archive-only. Built-in `--archive FILE` remains
a separate, strictly pinned offline-download path. Custom images preserve an
existing resolver configuration; a missing resolver uses the managed endpoint.
Image archives made by `docker save`/OCI layouts and disk images are not flattened
rootfs archives; use an unpacked rootfs or a `docker export` tar instead. URLs,
layer composition, xz and zstd are not accepted by this first custom-import path.

`--init systemd` on a custom rootfs requires systemd already installed in that
image; Trames does not assume its package manager or install Debian packages into
it. Only the original Debian service environment has current boot acceptance.
Import/restore keep custom environments and their commands portable. A failed
import never publishes an environment or replaces an existing one.

The low-level runner also accepts `--rootfs TAR_OR_DIRECTORY --storage NEW_DIR`;
it only initializes a fresh store and never overlays an existing one. Without
`--storage`, it retains the smaller in-memory VFS capacity. Normal product use
should create a named environment.

## Optional systemd service environment

Shell is the default and has no init-manager startup or resident service cost.
Select the boot mode per environment in the manager's Create/Settings dialogs,
or when deploying from the command line:

```sh
trames env create services --distro debian --init systemd
# Or opt in on an existing, stopped Debian environment:
trames env configure debian-work --init systemd
trames env start services
```

Opt-in installs Debian's original `systemd` and `dbus` packages using apt, with
no recommended desktop packages. Network access and Debian repository trust are
required for this first setup. It is idempotent and is not repeated at each boot.
A failed setup does not publish the new init setting. Old configurations without
an `init` field continue to use shell; Alpine currently accepts shell only. Custom images may opt into systemd if
their original systemd is already installed; no package setup is assumed for them.

Systemd runs as guest PID 1 and boots `multi-user.target`. The foreground terminal
is a local root shell managed by `console-getty.service`, without a separate Linux
password, matching shell mode's access. Enable normal services with
`systemctl enable --now NAME`; enabled multi-user services start on subsequent
boots. `journalctl`, D-Bus, oneshot and notify services use the original binaries.
`trames: session ready` means the execution session exists; `trames: systemd ready`
comes from PID 1's actual READY notification. Wait for the shell prompt before
entering commands. One foreground session owns an environment; a second terminal
cannot attach to a live init yet. Exiting the root shell respawns its console.

`env stop NAME` waits for PID 1's signal-handler notification and requests a clean
Linux halt. Services stop and filesystems sync/unmount before the private store
is released. It never shuts down macOS. `systemctl halt` inside Linux also works;
guest restart/kexec/suspend remain unsupported. Keep the terminal open until
shutdown completes. Explicit `--timeout` still cancels a test session at its bound.

For stopped-environment maintenance, override the selected mode explicitly:

```sh
trames env start services --init shell -- /bin/sh -l
trames env configure services --init shell     # Keep packages and guest files
```

Systemd boot takes no separate Linux command. Mode changes require a stopped
environment and never reset the store. Backup/restore preserve the init choice. Import and restore create a new
machine identity, so a clone can run alongside its source; the source stays
unchanged. Normal restarts preserve machine identity.
Fresh per-boot `/run`, device and shared-memory mounts grow on demand without
a fixed tmpfs byte/inode quota or preallocation. They remain guest VFS objects;
no host mount or privileged daemon is created. Windows/compute remain opt-in
product services and do not depend on systemd. A window-enabled root console
receives the usual Wayland environment.

This option provides **service management**, not a full Linux seat/login stack.
Trames owns the local terminal and macOS owns physical devices. A per-boot,
explicit `ConditionVirtualization=!container` drop-in skips `systemd-logind`:
its shared-mount propagation dependency is not implemented by the common VFS.
No other service's sandbox or syscall restrictions are disabled. Units requiring
unsupported Linux facilities still fail with their real errors. The startup
policy follows systemd's [container interface](https://systemd.io/CONTAINER_INTERFACE/).

Run the focused, network-enabled acceptance on Apple Silicon against a disposable
registry (it never changes existing environments):

```sh
python3 tests/integration/mac-systemd.py --runner target/release/trames-mac-run \
  --debian /path/to/verified-debian-rootfs.tar.gz --output target/systemd-acceptance
```

It checks original package provisioning, 1/4 CPU boots, zero failed units in the
scoped service environment, persistent enabled services, journal/notify behavior,
console input, graceful stop, terminal restoration, mode switching and backups.

## Settings and directory sharing

```sh
trames env configure work --cpus 4 --memory-mib auto --network outbound
trames env configure work --mount /work="$HOME/Projects"
trames env configure work --mount-ro /inputs="$HOME/Datasets"
trames env configure work --clear-mounts
trames env configure work --windows off       # Pure console/compute execution
trames env configure work --vulkan on --video on
```

Memory defaults to host-managed demand allocation; an explicit MiB value is a
hard cap. CPU settings are stored per environment. Native Vulkan/video switches
authorize those existing services; guest loader/ICD/Mesa/libva libraries and
applications are still required. Existing [graphics setup](../validation/mac-firefox-gpu.md#reproduce-and-run)
and [video scope](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/vaapi.md)
apply; enabling a service does not install a Debian-compatible graphics driver
or claim codec/conformance support. A base environment is usable without them.

Shared directories use the live native backend, not a synchronization mirror.
See [file sharing](file-sharing.md) for mmap, lock, inotify and database limits.
Removing an environment never removes its shared host directories.

## Lifetimes, interruption and storage

Sessions have **no default time limit**. `--timeout SECONDS` is an opt-in bound;
zero disables it. In shell mode, SIGTERM, SIGHUP and host SIGINT request
cancellation at the existing scheduler control hook, followed by storage sync
and terminal restoration. In systemd mode these request PID 1 shutdown; an
unavailable terminal/output transport may still interrupt that shutdown. Keyboard Ctrl-C/Ctrl-Z in a raw interactive terminal still reach
the Linux terminal/job-control model. SIGKILL cannot sync or restore a terminal;
use `stty sane` if an externally killed interactive process leaves raw settings.
Guest data explicitly synced before SIGKILL must remain readable after restart.

A managed session holds a per-environment lock through teardown. The underlying
store also retains its existing exclusive lock. A second start, backup, import
of a live store, deletion or settings change is refused while the environment
is busy. Stop connects to an environment-specific local control socket; it does
not kill a PID read from a stale file. The existing control hook checks pending
admin connections at most ten times per second; no resource service thread or
busy wait is introduced. Signal cancellation itself is one relaxed atomic flag.

Environments live under `~/Library/Application Support/Trames/environments/NAME`.
`TRAMES_HOME=/absolute/path` selects another registry for all commands, including
terminals launched by the manager. Configuration schema 1 is explicit; unknown
schemas are rejected without rewriting data. The disk manifest reader continues
to accept its existing versions 3/4. Older binaries cannot read manifest v4.
Keep a backup before upgrading; downgrade requires restoring a compatible backup.

Creation/import/restore populates a private staging directory and publishes it
with a same-directory rename only after validation and sync. New-rootfs
population defers repeated namespace manifests until one final commit; normal
runtime persistence is unchanged. Failed attempts are removed; a subsequent
operation on the same name reclaims unpublished stores left by a killed creator.
No automatic format reset or destructive repair is attempted on a damaged store.

## Back up, restore and import

```sh
trames env stop work
trames env backup work ~/Backups/work.trames.tar.gz
trames env restore work-restored ~/Backups/work.trames.tar.gz
trames env import existing-copy /path/to/stopped/legacy-store
trames env check work
trames env delete work-restored --yes
```

Backups stream native object contents and the committed manifest while holding
both management/store locks. They contain configuration and a SHA-256 inventory
for every file. Existing backup destinations are refused. Restore accepts only
known regular-file paths, verifies the complete inventory and gzip checksum,
opens the staged VFS using the normal manifest validation, and requires a new
name. Restore clears host directory grants: select those folders again on the
receiving Mac. Backups are not encrypted or authenticated; protect their files
as you would the original Linux environment.

Existing-store import creates a separate copy under a new name and preserves the
original store. It requires a stopped, valid Alpine or Debian store. Its DNS
and installed applications are preserved. There is no implicit relocation of an
existing Firefox/development store or migration of its guest libc.

A failed store check does not trigger a reset. Preserve the original directory
before investigating. For ENOSPC, free host disk space or increase the existing
private logical quota through the legacy storage tooling as appropriate; do not
remove individual objects or edit the manifest. Creation/restore may require
space for both the original and new copies. Backups do not include shared-folder
contents or process memory; this is offline file recovery, not live snapshots.

## Build and distribute

```sh
bash scripts/build-mac.sh                     # Dev channel, ad-hoc local build
python3 scripts/package-mac.py --unnotarized  # Dev ZIP, no notarization
# Stable channel, local unnotarized ZIP:
TRAMES_BUILD_CHANNEL=stable bash scripts/build-mac.sh
python3 scripts/package-mac.py --app target/release/Trames.app --unnotarized
# Developer ID / notarized stable distribution:
TRAMES_BUILD_CHANNEL=stable TRAMES_SIGN_IDENTITY='Developer ID Application: …' bash scripts/build-mac.sh
python3 scripts/package-mac.py --app target/release/Trames.app \
  --notarize-profile YOUR_EXISTING_PROFILE \
  --corresponding-source /path/to/native-driver-and-dependency-sources.tar.gz
```

Product channel and distribution signing are independent. A stable-channel local
package can be unnotarized; it still uses the stable app/CLI/data paths. Such ZIPs
have an explicit `-unnotarized` suffix and do not claim Gatekeeper acceptance.
Dev packages use a `Trames-Dev-` prefix. The old `--development` packaging flag
remains an alias for `--unnotarized`; it does not change a bundle's channel.

The build uses the workspace version for bundle metadata. Production packaging
requires a clean source tree, exact runtime pin, stable channel, hardened Developer
ID signature, corresponding native dependency sources, successful notarization,
stapling and Gatekeeper assessment. ZIPs include installer, license, guide, paired
project/runtime source exports, channel/build identities and file checksums. The
script does not publish GitHub releases or obtain account credentials.

Run `trames --version` or `trames env doctor` to collect product/runtime revisions,
rootfs versions/hashes and host capability limits. The source acceptance harness
`tests/integration/mac-environments.py` uses disposable environments to exercise
both distros, persistence, shared files, locks, backup checksums, traversal errors,
import, signals and crash/reopen. `--soak-seconds N` adds a longer repeated
read/write/reopen interval; it is opt-in and not a claim of elapsed soak coverage.
Real sleep/wake, power loss, a new Mac's first installation and multi-hour
application workloads need their own release acceptance evidence.
