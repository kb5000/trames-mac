# Native Mac environment manager

The manager is the AppKit presentation of the existing environment registry and
shared `trames_distro::environment` model. `manager.m` owns navigation, per-UUID
state, forms and task presentation; `manager-actions.m` owns user actions and
native creation/port editors. `manager-ui.h` is their UI-only boundary. The
explicit `TramesManagerLoadActions` reference retains Objective-C category
methods when these translation units are linked from a static archive.

`manager_model.rs`, private to `environments`, supplies the versioned `_manager`
snapshot/validate/save JSON protocol. These commands are internal UI interfaces,
not guest RPC devices or another execution backend. Field and capability
descriptors are declarative. Changes must use the shared manifest validation,
Mac backend validation and original TOML encoder. Registered identity and image
provenance are immutable through a form patch.

Each UUID has its own draft, base revision, task and session cache. Snapshots do
not overwrite an active native field editor. A revision hashes instance, explicit
and effective profile files, including absent files. Validate/save compare it
under the registry metadata lock and reject stale writes with the draft intact.
Saving never acquires the running guest lifetime lock or stops a CPU. Grants are
stored separately and are never implicitly enabled by configuration edits.
Malformed grants/networks produce an inspection error while unrelated
environments and their profiles remain visible; inspection does not repair files.

Long image preparation runs under the existing per-name management lock. The
registry metadata lock covers only initial defaults inspection and final
publication, so downloading/installing does not block every environment's cold
configuration work. Dot-prefixed staging directories are not registered
identities. Publication rechecks the name and UUID under the metadata lock.

Existing `TramesTaskStream` asynchronously drains child output on the main queue.
The native regression links the same static archive shape as production and
checks independent tasks, visible output before exit, UTF-8 tails, quiet snapshots
and resource cleanup. The UI retains a bounded visible tail; it does not redirect
guest syscall/device work through the manager. Configuration refresh uses vnode
notifications on registry/selected directories, activation and own-operation
completion, without timers, busy loops or new service threads. Normal guest
memory, network and file access paths are unchanged.

Future fields belong in the shared configuration model and cold descriptors;
platform-specific controls belong in the product UI. Native CPU interception
remains the high-throughput graphics/video/thunk path. `/dev/trames/*` remains
low-rate guest control. Shared files retain the original hostfs mount path.
