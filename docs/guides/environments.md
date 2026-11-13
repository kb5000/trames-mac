# Environments and CLI

The host CLI selects an environment with `-e NAME_OR_UUID`. Without a command it
opens a shell when stdin is a terminal. `exec` passes argv literally, preserving
exit status and separate stdin/stdout/stderr pipes.

```sh
trames env create work --distro debian
trames -e work shell
trames -e work exec -- uname -a
printf hello | trames -e work -- cat
trames --list
trames --terminate work
trames --shutdown
```

Exiting a shell leaves a managed environment running. Repeated shell/exec calls
use independent sessions. Multiple environments may run concurrently. Service
identity, locks, sockets and stores use the UUID; the human name is an alias.

## TOML profiles

The shared [manifest and capability specification](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/environment-configuration.md)
describes Linux/Android, hardware, native components, session settings, shares,
and control permissions. There is no legacy config migration.

```sh
trames env create --config /absolute/path/work.toml
trames env create custom --rootfs /absolute/path/rootfs.tar.gz
trames env create android --distro android --rootfs /absolute/path/adapters.tar.gz
trames -e work config show
trames -e work config path
trames -e work config set resources.cpus all resources.memory 4GiB
trames -e work config set network.mode bridge network.bridge development
trames -e work config validate
trames config --global set default_environment work
trames config --global set defaults.components.vulkan false
```

Configuration changes apply on the next environment start. Multiple `set` pairs
are validated and saved together. CLI defaults live in `config.toml`. Installed
environments have host-owned `instance.toml` metadata and an optional explicit
`env.toml`; `effective.toml` is an inspection cache, not a second config source.
Without an explicit profile, startup reads guest `/etc/trames/env.toml`.

Defaults use all available execution CPUs, host-managed memory, native host
networking, and Vulkan. A memory size is a demand-backed limit, not a reservation.
Systemd preparation is explicit (`trames env setup work --init systemd`).
`components.thunking = "auto"` enables interpreter acceleration on iPad and
leaves ordinary instructions native on Mac HVF; forcing interpreter thunks on
HVF is rejected before startup.

## Private networks and shared folders

```sh
trames network create development
trames network create isolated --internal
trames network ls --json
trames network inspect development
trames -e work config set network.mode bridge network.bridge development
trames -e work config set network.publish '[{"protocol":"tcp","host":"127.0.0.1:8080","guest":80}]'
trames env share work project /Users/example/Projects /work rw
```

These are Trames networks; no macOS bridge or privileged network service is
installed. `--internal` disables upstream NAT. Named networks keep separate
private address spaces. Shared folders retain the existing hostfs data path.

## Guest controls

A requested capability also needs a host grant:

```sh
trames -e work config set interop.clipboard.enabled true
trames env grant work clipboard on
trames -e work config set interop.host_exec.enabled true
trames env grant work host-exec on
trames -e work config set interop.host_cli.enabled true
trames env grant work host-cli on
```

Restart to apply the grants. Disabled devices are absent; Clipboard, UI, CLI,
host execution, file, port, and mount permissions are independent. Inside Linux:

```sh
trames info
trames clipboard read
trames clipboard write '"clipboard text"'
trames host-cli -- --version
trames host-exec -- /usr/bin/uname -s
trames files
trames rpc port-forward ports.list '{}'
trames rpc mount-control mounts.add '{"share":"project","target":"/mnt/project"}'
```

Port/mount control also needs its matching manifest request and host grant.
Dynamic changes last for the current boot; edit TOML for persistence. Files move
through mounted folders, using normal Linux tools and the original file I/O
implementation. Control-device replies are JSON; host command replies stream in
bounded frames. The native Wayland clipboard continues to use its streaming
pipe backend.

## Storage and channels

`env backup NAME FILE`, `env restore NEW_NAME FILE`, and `env import-store NAME
DIRECTORY` preserve private storage while assigning independent identities.
Backups never transfer host grants. Stable and Dev use separate app identities,
registries, services and stores; `TRAMES_HOME` provides an explicitly isolated
registry for testing. Existing stores can receive separately authored new TOML
profiles without being re-imported or migrated by the application.

## Persistent Docker services

Create a Debian systemd environment with networking enabled. A headless service
needs no presenter or audio device:

```sh
trames env create docker --distro debian --init systemd --cpus 4 --network host \
  --vulkan false --vaapi false
trames -e docker config set display.windows false interop.clipboard.enabled false
trames env start docker
trames -e docker shell
```

Inside Linux, install Debian's original packages and enable the daemon:

```sh
apt-get update
apt-get install -y docker.io procps
systemctl enable --now docker

docker volume create web-data
docker run -d --name web --restart unless-stopped \
  --log-opt max-size=1m --log-opt max-file=3 \
  -p 127.0.0.1:8080:80 -v web-data:/usr/share/nginx/html nginx:alpine
docker exec web sh -c 'printf "Trames Docker is running\n" > /usr/share/nginx/html/index.html'
```

On the Mac, `curl http://127.0.0.1:8080/` accesses the container. Close or detach
all terminals and the service remains running. `trames -e docker shell` opens
another independent shell. Use `trames env enable docker` for this macOS
user's next login, and `trames env stop docker` for a graceful systemd shutdown.
Stopping the environment first also permits `trames env backup docker ARCHIVE`.
Named Docker volumes, images, packages and enabled services live in its private
store and survive a clean environment restart.

Acceptance uses Docker 26.1.5, containerd 1.7.24, runc 1.1.15 and nginx 1.31.8
with the default master plus four worker processes. The actual driver reports
`overlay2`, `systemd` and cgroup v2. Tests cover an official image pull, host HTTP,
volume writes, Docker-daemon restart, whole-environment stop/start, unsuccessful
host-process restart and launch-agent bootstrap. No host Docker, privileged host
helper, full Linux-kernel VM, daemon security override or single-process nginx
configuration is used. Login-agent bootstrap is tested without logging out the
macOS user; services are unavailable while the Mac sleeps or before user login.

This remains a userspace Linux kernel, with explicit compatibility limits.
Kernel modules and privileged physical-device operations are unavailable. The
current cgroup controllers do not provide Docker CPU/memory/I/O resource limits;
a successful daemon start does not claim support for those limits. FASYNC is
currently implemented for pipes, Unix sockets and PTYs. Legacy AIO supports
buffered file operations using the existing disk pool, not O_DIRECT DMA.
Other images may require additional Linux interfaces. Use native host directory
mounts only when explicitly configured; private Docker volumes are the tested
persistent-service path.


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

## Native audio

Playback and capture are independent host opt-ins. `--audio` enables playback;
`--audio-input` also enables capture and requires the normal macOS microphone
permission. Persist these for an environment with:

```sh
trames-dev -e work config set components.audio true components.audio_input true
```

Install the paired native ALSA plugin in that Linux environment and configure
ALSA's default PCM to use it. Build it with the integration repository's
`scripts/build-audio-guest.sh` on ARM64 Linux. The standard ALSA `plug` layer
provides format conversion; no PulseAudio/PipeWire server is required.
For the graphics test launcher, `--audio-plugin PATH` imports the plugin and
an isolated ALSA configuration; add `--audio-input` to enable the microphone.
The launcher preserves explicit existing cubeb preferences.

`scripts/run-firefox-gpu.py --host-profile /path/to/dedicated-linux-profile`
uses the existing shared-directory backend for a persistent Linux Firefox
profile. Use a dedicated directory, separate from native macOS Firefox profiles.
The guest profile path is selected by `--profile`. Without `--host-profile`, the
launcher keeps using private guest storage. This option does not migrate or
replace an existing private profile. See the audio/playback validation notes
for the measured difference during background database saves.

## Application language

The native manager, dialogs, window menus and microphone permission explanation
support English and Simplified Chinese. They follow macOS's per-application
language preference; other languages fall back to English. Environment names,
guest window titles, guest output and CLI/package diagnostics retain their source
text. App language does not change Linux LANG or LC_*; the existing C.UTF-8
default needs no additional locale installation.

The current shared-kernel coordinator dependencies and measured migration
priorities are documented in the
[coordinator audit](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/coordinator-removal.md).
Use `--metrics` with the executor for the existing `execution paths` JSON counters;
profiling is disabled during normal use.

## Display density and host sleep

Wayland windows publish their current screen's integer density through output
and surface-scale events. Moving a window between displays updates its density;
guest geometry and input remain in logical units. Fractional scaling and physical
monitor geometry are not exposed by these virtual outputs.

The native presenter cancels outstanding frames and releases pressed input on
host sleep. Wake invalidates static frames, refreshes window size/density and
schedules the existing run loop. Guest RAM and CPUs are retained. The shared
runtime now distinguishes awake monotonic time from sleep-inclusive boottime and
uses native sleep-aware boottime timerfds. Validation used simulated clock jumps
and native timers; it did not put the host to sleep. Existing realtime-based wait
and wall-clock adjustment behavior remains outside this change.

See the [shared display lifecycle design](https://github.com/kb5000/trames-runtime/blob/main/docs/architecture/display-lifecycle.md)
and [paired platform validation](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/display-lifecycle-20261104.json).


## Optional Android adapters

Android is a separate experimental target. Import the small Trames ARM64
adapter archive; first installation obtains the pinned official Waydroid
Android 13 system/vendor ZIPs directly from upstream and verifies their sizes
and SHA-256 hashes. The adapter contains only Vulkan/Gralloc frontends, the
64-bit audio-service launcher, graphics properties and provenance. Matching
source exports are distributed separately. No Android image or APK is bundled
into the Mac application or adapter download.

```sh
trames env create android --distro android --rootfs Trames-Android-13-arm64-adapters.tar.gz
trames env start android
trames -e android apk install /path/to/application.apk
trames -e android apk launch com.example.application
```

Wait for first boot/user unlock before launching applications. ZIPs are cached
under `TRAMES_HOME/cache/android`; `TRAMES_ANDROID_IMAGE_CACHE` can point at an
existing directory of the exact pinned ZIPs for offline installation. Each
environment keeps its own writable store. Image conversion runs only during
installation, without root, filesystem mounting or a Linux VM. Old full Android
rootfs archives remain supported. Host networking is the default; use `network.mode = "none"` for a disconnected guest.
Installation downloads are separate from guest networking.
Host audio and physical iPad Android acceptance remain pending.

The [shared Android architecture record](https://github.com/kb5000/trames-ipad/blob/main/docs/architecture/android-bringup.md#small-adapter-package-and-upstream-composition)
describes the component contract and current compatibility boundaries.
