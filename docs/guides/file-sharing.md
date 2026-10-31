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

## Running programs and development tools

Use Linux ARM64 programs in the shared tree. For example, with a compiler
installed in the selected Linux environment:

```sh
target/release/trames-mac-run --storage ./linux-store \
  --mount "/work=$HOME/TramesShared" /bin/sh -lc \
  'cd /work; cc main.c -o app; ./app'
```

Direct script launch also handles `#!`, its single optional interpreter argument
and bounded recursive interpreters. Execute permission and `noexec` mounts apply.
A read-only mount permits execution and private writable COW mappings, while
native file writes remain denied. A macOS Mach-O binary is not a Linux executable.

The [workspace fixture](https://github.com/kb5000/trames-ipad/blob/main/tests/fixtures/hostfs-workspace.c)
and [acceptance runner](https://github.com/kb5000/trames-ipad/blob/main/tests/integration/hostfs-workspace.py)
exercise a native shared development tree. GCC, its support programs and the
Linux system libraries also run from read-only shared `/usr` and `/lib` trees;
this is not the Mac host's `/usr` or `/lib`.

| Shared-directory workflow | Local result |
| --- | --- |
| Compile a static ELF, dynamic ELF and plugin `.so` with guest GCC | Passed |
| Run ELF with its loader and dlopen library also in the shared tree | Passed |
| Direct shebang script, guest exec, arguments and executable symlink | Passed |
| Read-only mount execution and MAP_PRIVATE COW without file changes | Passed |
| Private mapping fork isolation, partial EOF zero fill and past-EOF SIGBUS | Passed |
| Git init/add/commit/gc/status | Passed |
| Python module import, ctypes library load and SQLite default journal mode | Passed |
| Execute-bit denial and guest noexec remount | Passed |

Native create/open retains its original descriptor, so a creator can finish
writing a newly created read-only object file as Git requires. Closing the latest
same-access handle does not lose earlier open descriptions. Executable and map
capabilities keep the original native inode across host atomic replacement.
See the [workspace validation record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/hostfs-workspace.json).

## Native file events and advisory locks

External host changes now wake guest inotify readers, poll and epoll. File
content/attribute changes and immediate directory create/delete/rename are
supported. Watches are allocated on demand and drained by an existing executor;
there is no background watcher thread or periodic directory scan. A namespace
watch retains only the watched directory descriptor. Monitoring child content
or attributes also retains descriptors for its current immediate children.
Removing watches or closing the inotify instance releases those interests.

macOS vnode events coalesce: rapid external create/delete sequences may disappear
before a snapshot, and moves across separate directories may be delete/create.
External open/access/close history is unavailable. Guest-created notifications
retain their existing behavior; host echoes are suppressed. This supports many
reload/build workflows, but is not complete Linux inotify history conformance.

Whole-file `flock` now coordinates with independent native processes. Shared and
exclusive modes, nonblocking conflicts, blocking host unlock wakeups, dup/fork
and final-close lifetime are supported. Private mappings also retain the OFD
lock until unmapped. Byte-range `fcntl` locks remain guest-local; do not use them
to coordinate a database concurrently opened by native tools. SQLite's default
journal smoke test covers guest-only use, not cross-boundary database locking.

`MAP_SHARED` still returns ENODEV, including SQLite WAL's shared-memory file.
Supporting it requires coherent file-backed aliases plus recovery that confines
external-truncate SIGBUS to the guest. Neither copied pages nor delayed writeback
provide that contract. MAP_PRIVATE and ordinary native reads remain available.

The [interop fixture](https://github.com/kb5000/trames-ipad/blob/main/tests/integration/hostfs-interop.py)
checks independent host/guest processes with 1 CPU and 4 CPUs plus a running guest
peer. Native create wakes blocking read, poll and epoll; host lock release wakes
guest flock, and a guest lock remains visible natively after dup/close. Existing
GCC, Git, Python, shared-library and SQLite default-journal workflows also pass.

## Scope and limits

Shared directories can hold Linux ARM64 scripts, executables, dynamic loaders
and libraries, as well as sources and outputs. This is not a complete POSIX
filesystem conformance claim. MAP_PRIVATE supports demand-read executable/library
pages and private COW data. Host MAP_SHARED returns ENODEV; applications that
require coherent shared mappings (including SQLite WAL use) remain outside the
supported shared-directory scope. Extended attributes and special host
files/devices/sockets are unsupported. External inotify notifications and native
flock have the scope described above; byte-range locks remain guest-local.
Normal native read/write, positioned I/O, append, sparse size, fsync, chmod,
ownership attempts, timestamps, hard links and rename use the real host objects.
Native I/O and volume errors remain applicable.

Private mapped pages load from the retained inode only when reached. Writes use
the shared guest COW model and never write a private page back to the host file.
They do not replace the ordinary inode's live native read path. Guest-known size
changes update EOF metadata without polling on every CPU load. Already resident
private pages do not monitor external host edits/truncation; unmap/remap to obtain
a new view when coordinating such changes. This is not a coherent host mapping
or an external-mutation fault protocol.

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
operation counts are checked. Results are three-run medians. Initial sharing results remain in the
[hostfs record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/hostfs.json).
The retained-handle/private-byte-accounting follow-up is recorded in the
[private I/O record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/private-io.json),
including raw observations, run ranges and the longer five-pair idle-mount check.

The Linux native baseline uses an ext4 disk in a 6-vCPU/8-GiB ARM64 VZ VM. Mac
native and Trames use APFS on Apple M6/macOS 27.0.1, 24 GiB host RAM. The Linux VM
is stopped during Mac measurements and no compilation/sampler runs concurrently.
This compares complete I/O paths, including different kernels, libraries,
filesystem layouts and pathname traversal. Cached GiB/s is not SSD throughput,
and these measurements do not establish cold-I/O or physical power-loss durability.

| Fixed work | Private before | Private after | Host mount | macOS native | Linux VM native |
| --- | ---: | ---: | ---: | ---: | ---: |
| 64 KiB reads, MiB/s | 3,798 | 8,564 | 8,095 | 40,793 | 34,757 |
| 64 KiB writes, MiB/s | 2,220 | 7,918 | 6,860 | 26,740 | 44,797 |
| 4 KiB reads, MiB/s | 355 | 1,786 | 1,434 | 11,696 | 13,798 |
| Path stat, operations/s | 152,805 | 168,627 | 27,290 | 1,824,551 | 3,800,210 |

Private open descriptions now retain native file leases. Repeated reads/writes
reuse the selected inode handle; closed cached inodes retain no native FD.
Private logical-byte accounting updates a store-local atomic total only on size
changes and final backing release. Byte admission no longer enumerates every
inode or counts nodes. Existing quotas, guest permission checks and explicit
fsync/manifest durability remain in force. Memory filesystems keep their existing
accounting; there is no new service thread or global I/O lock.

This improves private data throughput 2.25x/3.57x/5.03x for the three data cases,
respectively, and all three now exceed the shared-directory path. Repeated host
path metadata resolution remains substantially slower. Native cached I/O also
has much lower overhead, so these results do not imply native I/O parity.
Private final fsync still takes about 15–17 ms in these runs because it preserves
namespace checkpoints; the record retains those times outside the main interval.

With an idle shared mount, private 4 KiB reads measure 1,783 versus 1,786 MiB/s
without one in the three-repeat run. A longer shuffled check on the final binary,
five runs per variant and 300,000 reads per run, measures 1,797.588 MiB/s without
and 1,816.583 MiB/s with one (+1.06%). No ordinary-read regression is observed in
this local check; run ranges are preserved, and this is not a zero-cost proof.

Build the fixture with assertions enabled (`clang -O2` on macOS, `gcc -O2 -static`
on ARM64 Linux). Run each mode (`read64k`, `write64k`, `read4k`, `stat`) on an
isolated file. The fixture accepts `FILE MODE OPERATIONS` and emits JSON. Use the
same Linux executable in the VM and Trames. The Python runner accepts `--runner`,
`--baseline`, `--program` (Linux ELF), `--native` (Mac executable), `--rootfs`,
`--output` and optional `--linux-results`; its `--help` describes the isolation
requirements. The machine-readable record preserves native Linux observations,
compiler flags, operation counts, binary hashes and separate fsync times.

### Native interop follow-up (2026-10-31)

The [interop validation record](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/hostfs-interop.json)
preserves the shuffled controls, final three-run medians and all raw observations.
The native Linux executable is identical to the one run in Trames. Final guest
acceptance includes 12 process cases, covering close/FD reuse during a blocked
read as well as the 1-CPU and concurrent 4-CPU paths.

| Cached fixed work | Shared before | Shared now | Content watch | Held flock | Mac native | Linux VM native |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 KiB reads, MiB/s | 7,803 | 7,976 | 7,833 | 7,920 | 39,244 | 38,431 |
| 64 KiB writes, MiB/s | 7,240 | 7,251 | 7,190 | 7,222 | 26,285 | 45,558 |
| 4 KiB reads, MiB/s | 1,381 | 1,417 | 1,390 | 1,409 | 11,154 | 13,912 |
| Path stat, operations/s | 21,949 | 21,824 | 21,829 | 21,979 | 902,894 | 3,937,550 |

A content watch costs about 0.8–1.9% in these data cases. Held-lock throughput
changes by 0.4–0.7%; ordinary byte I/O does not query lock state. No new global
filesystem service, thread or per-read native notification check is introduced.
Only actual pending native resources enter the joined executor's wait set, so
an unread notification on an unrelated descriptor cannot cause idle spin.

Short private read64k medians varied between sweeps. A longer check uses five
randomized pairs and 8 GiB per run: 8,653 MiB/s before and 8,744 after (+1.05%).
This does not show a sustained private-read regression; the record retains all
observations rather than treating short-run differences as a speedup. Current
private write64k and read4k medians are 7,913 and 1,772 MiB/s. Native cached I/O
still has substantially lower overhead. Repeated shared path-stat resolution
remains the largest measured metadata gap and is not fixed by event/lock support.
