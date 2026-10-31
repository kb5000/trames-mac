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
`~/.local/bin/trames`. Add `~/.local/bin` to PATH if your shell does not already
include it. An existing unrelated `trames` executable is preserved. Upgrades
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
layout. Debian support means a shell and normal userspace programs, not booting
systemd, loading Linux kernel modules, or complete Linux ABI compatibility.

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
zero disables it. SIGTERM, SIGHUP and host SIGINT request cancellation at the
existing scheduler control hook, followed by storage sync and terminal
restoration. Keyboard Ctrl-C/Ctrl-Z in a raw interactive terminal still reach
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
bash scripts/build-mac.sh                       # Ad-hoc local development build
TRAMES_SIGN_IDENTITY='Developer ID Application: …' bash scripts/build-mac.sh
python3 scripts/package-mac.py --development   # Explicitly labelled local ZIP
python3 scripts/package-mac.py \
  --notarize-profile YOUR_EXISTING_PROFILE \
  --corresponding-source /path/to/native-driver-and-dependency-sources.tar.gz
```

The build uses the workspace version for bundle metadata. Production packaging
requires a clean source tree, exact runtime pin, hardened Developer ID signature,
corresponding native dependency sources, successful notarization, stapling and
Gatekeeper assessment. ZIPs include installer, license, guide, paired project/
runtime source exports, build identities and file checksums. Development archives
are labelled and do not claim Gatekeeper acceptance or stable-release validation.
The script does not publish GitHub releases or obtain account credentials.

Run `trames --version` or `trames env doctor` to collect product/runtime revisions,
rootfs versions/hashes and host capability limits. The source acceptance harness
`tests/integration/mac-environments.py` uses disposable environments to exercise
both distros, persistence, shared files, locks, backup checksums, traversal errors,
import, signals and crash/reopen. `--soak-seconds N` adds a longer repeated
read/write/reopen interval; it is opt-in and not a claim of elapsed soak coverage.
Real sleep/wake, power loss, a new Mac's first installation and multi-hour
application workloads need their own release acceptance evidence.
