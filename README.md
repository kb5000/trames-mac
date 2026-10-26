# Trames Mac

Apple Silicon hardware execution product for the shared
[Trames Linux userspace runtime](https://github.com/kb5000/trames-runtime).
ARM64 user instructions execute through Hypervisor.framework and a small EL1
exception adapter; no complete Linux kernel is booted.

```sh
git clone --recurse-submodules https://github.com/kb5000/trames-mac.git
cd trames-mac
bash scripts/build-mac.sh
target/release/trames-mac-run --rootfs /path/to/verified-Alpine.tar.gz /bin/busybox echo hello
```

`runtime/` pins an exact common-runtime commit. Run `git submodule update --init`
after switching revisions. The build script uses a local ad-hoc Hypervisor
signature; it does not require the iPad development certificate or a JIT entitlement.

The current runner supports one process, private mappings and Linux signal
handlers. Common process/clone/exec scheduling, shared hardware mappings and
live-anonymous pressure paging remain integration work. A memory limit reserves
no fixed guest RAM image, and released physical pages are returned while the VM
remains alive. See `SOURCE.json` for extraction provenance.
