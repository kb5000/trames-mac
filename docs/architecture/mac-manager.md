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
stored separately and are never implicitly enabled by ordinary configuration
edits or profile import. The Host Interop page exposes one Allowed/Not allowed
choice per capability. Its explicit local `_manager permission` action updates
both portable enablement and the UUID-bound host grant under the metadata lock.
The displayed state is the intersection of both. Enablement is written before a
new grant; revocation is written before disabling the configuration. These are
separate files rather than an all-or-nothing transaction: publication/durability
errors are reported, the refreshed intersection reflects any published half,
and retry completes the operation. Other drafts and grants
remain independent; an own-action revision update preserves unrelated drafts.
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

## Native control and layout contract

The sidebar starts at 280 points, with a 250–380 point divider range. Its table
column follows available width and each row uses two single-line, truncating
labels. Manager scroll views use small overlay scrollers that hide when idle.
There is no persistent closing-manager hint in either footer.

Sheet root stacks use four-edge Auto Layout constraints rather than autoresizing
frames. The creation panel provides 620 × 600 points, and port editing uses the
same constraint helper. A flexible footer spacer keeps wizard action buttons
at their intrinsic widths. CPU, memory and the optional hard limit use editable
combo boxes: friendly automatic/all-core/no-limit choices map to the original
TOML values, CPU counts are integers, and custom resource values still pass the
shared validator. Login startup uses a native checkbox.

All boolean form values use `numberWithBool:`. Boxing a C comparison expression
with `@(...)` can create an integer NSNumber; JSON then carries 0/1 instead of
false/true. The strict configuration decoder must continue rejecting integers
for boolean fields. Shared-folder read-only values follow the same rule.

`tests/native/manager-layout.m` exercises the production static archive's actual
controls, JSON types, resource selection, login actions, permission dispatch and
sheet hit targets at default/minimum window widths. OS mouse dispatch additionally
runs when an active desktop is available; inactive sessions report that portion
as skipped and still exercise native actions and geometry. Disposable running
Linux integration checks validate boolean round-trips and permission updates
without restarting the guest. These changes do not alter execution coordination.
