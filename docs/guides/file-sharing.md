# Host directories and guest file access

The terminal-first Mac product offers live shared directories and explicit offline
access to a private Linux store. Neither requires FUSE, a FileProvider extension,
a background mirror or a presenter. Ordinary Linux files retain their existing
backend. The shared runtime implements the optional `hostfs` feature; the iPad
product keeps its existing Files/FileProvider integration by default.

## Live bidirectional sharing

Create a host folder and attach it when starting the Linux environment:

```sh
mkdir -p "$HOME/TramesShared"
target/release/trames-mac-run --storage ./linux-store \
  --mount "/mnt/shared=$HOME/TramesShared" /bin/sh -l
```

The target guest directory is created if needed. Inside Linux,
`printf 'from Linux\n' > /mnt/shared/example.txt` creates the actual native file.
Finder and host terminal tools can read or edit the same folder immediately;
ordinary guest reads and lookups observe host changes. Large files are streamed
rather than imported wholesale into guest RAM or the private object store.

Use `--mount-ro GUEST=HOST` for a read-only capability. A guest remount cannot
turn that grant writable. Repeat either option for more directories. Native
sources must already exist. Each mount lasts for the session and is not persisted
in the private store; a guest pathname alone never grants host access.

An open guest file retains the original native inode across replacement, rename
and unlink. Reopening the path sees a replacement. Directory descriptors and cwd
follow native directory renames within the granted tree. Relative symlinks work;
absolute symlink contents resolve from the Linux guest root. Native traversal
never follows a symbolic link out of the selected directory capability.

The host account is projected to the guest uid/gid that selected the mount
(default root). Native host ownership, ACL/access, case and filename rules still
apply. Guest root cannot bypass the native account's permissions.

## Internal guest directories, offline

Stop the session using the selected private store, then use:

```sh
target/release/trames-mac-run --storage ./linux-store --fs ls /root
target/release/trames-mac-run --storage ./linux-store --fs cat /etc/os-release
target/release/trames-mac-run --storage ./linux-store --fs get /root/project ./project
target/release/trames-mac-run --storage ./linux-store --fs put ./input.txt /root/input.txt
target/release/trames-mac-run --storage ./linux-store --fs put --replace ./edited.txt /root/input.txt
```

`get` and `put` support regular files, directories and symlinks. Copies use bounded
64 KiB buffers. Existing destinations are preserved; `put --replace` accepts a
regular file, stages a sibling, fsyncs it and atomically renames the new inode.
Copies do not preserve Unix ownership, timestamps or all native metadata.

These commands use the existing VFS and exclusive store lock, without creating
a VM, Linux process, window or server. An active store returns EBUSY. For live
exchange, copy the desired guest files into a shared directory from the running
Linux session. Do not edit the private store's object files or manifest directly.

## Scope and limits

This is a file-exchange backend, not a complete POSIX filesystem conformance
claim. Host-file mmap returns ENODEV; keep mapped dependencies and mutable
databases in the private Linux store. Extended attributes and special host
files/devices/sockets are unsupported. Guest inotify does not monitor external
host edits, and advisory locks do not coordinate with independent native writers.
Normal native read/write, positioned I/O, append, sparse size, fsync, chmod,
ownership attempts, timestamps, hard links and rename use the real host objects.
Native I/O and volume errors remain applicable.

There is no polling tree scan, startup import of the tree, global filesystem
service thread or per-frame synchronization. Unused mounts add no native file I/O
to ordinary private-file reads. Monotonic mount/target markers skip unrelated
mount-table scans. Only actual shared resources and namespace/cache updates are
synchronized; bulk reads reuse the existing bounded disk pool.

## Local validation and performance

The local checks cover bidirectional edits, retained old files after atomic
replacement and unlink, native directory rename with cwd/dirfd, read-only grants,
symlink confinement, O_PATH symlink lifetime, hard links, metadata, sparse 8 GiB
files, and an asynchronous bulk read through close/FD reuse. Concurrent metadata
admission and rename exercise cache lock ordering. The CLI also passes Unicode,
recursive/link copies, a 4 MiB streaming round trip, no-clobber and atomic upload.

The [shared benchmark fixture](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/hostfs-bench.c)
uses identical fixed work on macOS native, ARM64 Linux native in the VZ build VM,
and Trames. The [runner](https://github.com/kb5000/trames-ipad/blob/main/tests/integration/hostfs-perf.py)
compares the old private backend, current private files, an idle host mount and
actual host-backed files. All files have a cached 8 MiB working set. Initial
population/fsync, startup and final fsync are outside the main interval. Data and
operation counts are checked. Results are three-run medians; raw ranges and the
longer five-pair idle-mount check are retained in the
[machine-readable record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/hostfs.json).

The Linux native baseline uses an ext4 disk in a 6-vCPU/8-GiB ARM64 VZ VM. Mac
native and Trames use APFS on Apple M6/macOS 27.0.1, 24 GiB host RAM. The Linux VM
is stopped during Mac measurements and no compilation/sampler runs concurrently.
This compares complete I/O paths, including different kernels, libraries,
filesystem layouts and pathname traversal. Cached GiB/s is not SSD throughput,
and these measurements do not establish cold-I/O or physical power-loss durability.

| Fixed work | Private store | Host mount | macOS native | Linux VM native |
| --- | ---: | ---: | ---: | ---: |
| 64 KiB reads, MiB/s | 3,819 | 8,118 | 41,656 | 36,862 |
| 64 KiB writes, MiB/s | 2,225 | 7,095 | 26,901 | 44,959 |
| 4 KiB reads, MiB/s | 360 | 1,480 | 11,722 | 13,831 |
| Path stat, operations/s | 155,947 | 27,809 | 1,846,483 | 3,872,454 |

Compared with the pre-change private backend, ordinary reads/writes are within
1% in all three data cases; private path stat improves about 7%. Host mounts
improve data throughput roughly 2–4 times over the private store, but repeated
host path metadata resolution remains substantially slower. Native cached I/O
also has much lower overhead; file exchange does not imply native I/O parity.

The three-repeat test with an idle mount measured a 3.0% lower 4 KiB read median
(349 versus 360 MiB/s). A longer shuffled check on the final binary, five runs
per variant and 300,000 reads per run, measured 356.855 MiB/s without a mount and
360.949 MiB/s with one (+1.15%). This does not establish a consistent ordinary
read regression. Both observations and all run ranges remain in the record;
these short local trials do not prove literally zero dispatch cost.

Build the fixture with assertions enabled (`clang -O2` on macOS, `gcc -O2 -static`
on ARM64 Linux). Run each mode (`read64k`, `write64k`, `read4k`, `stat`) on an
isolated file. The fixture accepts `FILE MODE OPERATIONS` and emits JSON. Use the
same Linux executable in the VM and Trames. The Python runner accepts `--runner`,
`--baseline`, `--program` (Linux ELF), `--native` (Mac executable), `--rootfs`,
`--output` and optional `--linux-results`; its `--help` describes the isolation
requirements. The machine-readable record preserves native Linux observations,
compiler flags, operation counts, binary hashes and separate fsync times.
