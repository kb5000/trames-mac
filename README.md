<p align="center">
  <img src="apps/macos/Resources/Trames.png" alt="Trames Mac icon" width="128">
</p>

<h1 align="center">Trames Mac</h1>

<p align="center">
  <strong>A hardware-accelerated Linux workspace for Apple Silicon, powered by a userspace kernel and HVF.</strong>
</p>

<p align="center">
  Run unmodified ARM64 Linux binaries with near-native speed, seamless macOS window integration, and a unified CLI.<br>
  Zero guest Linux kernel overhead—system calls are intercepted and served directly in userspace.
</p>

<p align="center">
  <a href="https://github.com/project-trames/trames-mac/releases/latest"><img src="https://img.shields.io/github/v/release/project-trames/trames-mac?style=flat-square&amp;color=f06b45" alt="Latest release"></a>
  <a href="https://github.com/project-trames/trames-mac/actions/workflows/macos.yml"><img src="https://github.com/project-trames/trames-mac/actions/workflows/macos.yml/badge.svg?branch=main" alt="Mac CI"></a>
</p>

<p align="center">
  <a href="#installation"><img src="https://img.shields.io/badge/macOS-Apple%20Silicon-30303d?style=flat-square&amp;logo=apple&amp;logoColor=white" alt="macOS on Apple Silicon"></a>
  <a href="docs/architecture/execution-backends.md"><img src="https://img.shields.io/badge/execution-HVF-7c5269?style=flat-square" alt="Hypervisor.framework execution"></a>
  <a href="#features"><img src="https://img.shields.io/badge/guest-ARM64-f0ad43?style=flat-square" alt="ARM64 Linux guest"></a>
  <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/Rust-nightly-b65437?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust nightly"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-7c5269?style=flat-square" alt="GPL-3.0-or-later"></a>
</p>

<p align="center">
  <a href="https://github.com/project-trames/trames-mac/releases/latest">Download</a> ·
  <a href="#features">Features</a> ·
  <a href="#quickstart">Quickstart</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#building-from-source">Building</a> ·
  <a href="#documentation">Documentation</a>
</p>

---

## Features

Trames Mac bridges Linux command-line workflows and graphical applications directly into macOS without booting an actual guest OS kernel:

* **Near-Native Hardware Execution**: ARM64 user instructions run directly on host CPU cores via Apple's `Hypervisor.framework` (HVF), passing syscalls and page faults to the userspace Rust kernel.
* **Native Cocoa Windows**: Built-in Wayland and Xwayland compositing creates native macOS windows with smooth display scaling, trackpad gestures, clipboard sharing, and IME input.
* **Metal-Bridged Graphics**: Seamlessly maps guest Vulkan and OpenGL calls to Apple Metal for hardware-accelerated GUI apps and 3D rendering.
* **Isolated Persistent Environments**: Run multiple Alpine, Debian, or custom ARM64 rootfs instances concurrently with persistent disk state, optional systemd init, and scoped Docker daemon support.
* **Ergonomic Host CLI**: Control the full workspace lifecycle with the `trames` command—attach to PTYs, stream binary I/O, execute isolated commands, and forward ports.
* **Host Resource Sharing**: Transparent bidirectional host-guest file sharing, low-latency private bridge networking, and granular directory mount policies.
* **Headless Background Services**: Run background daemons and Vulkan compute workloads without opening GUI windows.

---

## Architecture

Unlike traditional virtual machines that boot a guest Linux kernel inside QEMU or Lima, Trames Mac uses HVF strictly as an **execution sandbox**:

```text
┌─────────────────────────────────────────────────────────────┐
│                 macOS Host (Apple Silicon)                  │
│       Native Window Server · Host Filesystem · Metal API    │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────┴──────────────────────────────┐
│                    trames-mac (Workspace)                   │
│         CLI / GUI Manager · Cocoa App Host · Lifecycle      │
│   ┌─────────────────────────────────────────────────────┐   │
│   │             trames-runtime (Userspace Kernel)       │   │
│   │     VFS · Futex · Processes · Userspace Net Stack   │   │
│   └──────────────────────────┬──────────────────────────┘   │
│                              │ Guest Syscalls / Exceptions  │
│   ┌──────────────────────────┴──────────────────────────┐   │
│   │             trames-hvf (Execution Engine)           │   │
│   │     Apple Hypervisor.framework Stage-2 Translation  │   │
│   └──────────────────────────┬──────────────────────────┘   │
└──────────────────────────────┼──────────────────────────────┘
                               │ Direct CPU Execution (ARM64)
┌──────────────────────────────┴──────────────────────────────┐
│                  Unmodified Linux Binaries                  │
└─────────────────────────────────────────────────────────────┘
```

* **Execution Boundary**: Instructions execute bare-metal on Apple Silicon. When a guest binary invokes a Linux syscall (e.g., `sys_clone`, `sys_epoll_wait`) or triggers a page fault, HVF returns control directly to `trames-runtime` to resolve it.
* **Shared Ecosystem**: The core Linux personality is identical across all Trames targets; only the execution engine differs ([trames-ipad](https://github.com/project-trames/trames-ipad) uses an interpreter; [trames-linux](https://github.com/project-trames/trames-linux) uses KVM).

---

## Installation

### Requirements
* Mac with Apple Silicon (M1/M2/M3/M4 series).
* macOS 14.0 (Sonoma) or later recommended.

### Quick Install

1. Download the latest Mac ARM64 `.zip` archive from [Releases](https://github.com/project-trames/trames-mac/releases/latest).
2. Extract the archive and run the included `install.command` from the extracted directory.
3. Restart your terminal session or reload your shell profile to ensure `trames` is in your `$PATH`.

---

## Quickstart

### Environment Management

Create and enter an Alpine Linux environment:

```sh
# Create a new environment named 'work'
trames env create work

# Spawn an interactive shell
trames -e work shell
```

Run isolated commands or launch the native Mac management dashboard:

```sh
# Execute a command and stream output back to host stdout
trames -e work exec -- uname -a

# Inspect environment configuration
trames -e work config show

# Open the Cocoa GUI dashboard
trames gui
```

Create a Debian environment with `systemd` init enabled for background services:

```sh
trames env create services --distro debian --init systemd
```

### Host Sharing & File Transfers

Mount host directories directly into the guest environment:

```sh
# Mount ~/Projects into /workspace inside the 'work' environment
trames env share work projects ~/Projects /workspace rw
```

Copy files bi-directionally between host and guest:

```sh
# Host to guest
trames -e work files cp ./config.yaml guest:/etc/app/config.yaml

# Guest to host
trames -e work files cp -r guest:/var/log/app ./logs
```

---

## Building from Source

### Prerequisites

* Apple Silicon Mac with Xcode and macOS SDK installed.
* Rust nightly toolchain (specified in [`rust-toolchain.toml`](rust-toolchain.toml)).
* Python 3 and native build essentials.

### Build

The default build requires a verified patched macOS MoltenVK static archive. Follow the [GPU build recipe](docs/validation/mac-firefox-gpu.md#reproduce-and-run) and set its directory on the first build; subsequent builds reuse the saved configuration.

```sh
git clone --recurse-submodules https://github.com/project-trames/trames-mac.git
cd trames-mac

# Build the development application bundle and sign with HVF entitlements
TRAMES_MOLTENVK_STATIC_DIR=/path/to/verified/macos-driver bash scripts/build-mac.sh

# Launch the built management application
target/release/trames-mac-run --manager
```

For building with specific options:
```sh
# Build the production stable channel
TRAMES_BUILD_CHANNEL=stable bash scripts/build-mac.sh

# Build without embedded MoltenVK graphics archive (uses external dynamic driver)
TRAMES_VULKAN_LINKAGE=external bash scripts/build-mac.sh
```

### Local Hardware Verification

Validate HVF execution and memory-pressure handling on your local machine:

```sh
python3 scripts/check-hardware.py
```

---

## Documentation

| Topic | Guides & Specifications |
| :--- | :--- |
| **Getting Started** | [Environment Lifecycle & Backups](docs/guides/environments.md) · [File Sharing Guide](docs/guides/file-sharing.md) |
| **Architecture** | [HVF Execution Backend](docs/architecture/execution-backends.md) · [Task State Machine](docs/architecture/task-state.md) |
| **Native Integration**| [Mac Manager & UI Architecture](docs/architecture/mac-manager.md) · [Native Windows](docs/validation/mac-windows.md) |
| **Graphics & Acceleration** | [Metal / MoltenVK Acceleration](docs/validation/mac-firefox-gpu.md) |
| **Containers & Daemons** | [Persistent Docker Services](docs/guides/environments.md#persistent-docker-services) · [Systemd & Docker Validation](docs/validation/mac-docker-gpu-20261113.md) |
| **Validation & Benchmarks** | [Validation Records](docs/validation/) · [CoreMark Performance Analysis](docs/validation/mac-compat-coremark.md) |

---

## Community and Contributing

We welcome contributions, bug reports, and hardware benchmark reports. Please review our [Contribution Guidelines](https://github.com/project-trames/.github/blob/main/CONTRIBUTING.md) and [`AGENTS.md`](AGENTS.md) before submitting pull requests.

* **Issues**: [GitHub Issue Tracker](https://github.com/project-trames/trames-mac/issues)
* **Security**: For private vulnerability reports, see [SECURITY.md](https://github.com/project-trames/.github/blob/main/SECURITY.md)
* **Conduct**: We adhere to the [Project Code of Conduct](https://github.com/project-trames/.github/blob/main/CODE_OF_CONDUCT.md)

---

## License

Trames Mac is licensed under the **GNU General Public License v3.0 or later** ([GPL-3.0-or-later](LICENSE)).<br>
Third-party component notices and provenance information are detailed in [SOURCE.json](SOURCE.json).

---

<p align="center">
  Part of <a href="https://github.com/project-trames">Project Trames</a> · A portable userspace Linux runtime with pluggable execution engines.
</p>
