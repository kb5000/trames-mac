# Mac manager corrections — 2026-11-16

The manager now uses a wider environment list, compact overlay scrollers and a
single host-permission choice. Creation and accessory sheets use constrained
root layouts; resource settings have editable presets and login startup uses
checkboxes. Checkbox, component-switch and shared-mount read-only values are
serialized as JSON booleans, fixing `invalid type: integer 0, expected a boolean`.

The sidebar starts at 280 points rather than 210, allows 250–380 points, and its
table column expands to the available width. Environment names and subtitles
use single-line truncation. Both persistent closing-manager hints are removed.
The new-environment panel provides 620 × 600 points with its content pinned to
all four edges. Its source, configure and review stages preserve clickable
button bounds at both default and minimum manager widths.

CPU choices include all active host cores and numbered presets. Memory includes
automatic shared host memory and common sizes; the optional hard limit has an
explicit no-limit choice. Custom values remain editable and pass the original
validator. Friendly labels map back to the original TOML values, including
integer CPU counts, `all`, `auto` and an absent optional limit.

The Host Interop page displays only Allowed/Not allowed. An explicit local
permission action updates both the portable enablement flag and the separate
UUID-bound host grant. Ordinary configuration saving/import still grants no
host access. Enabling saves configuration before granting; disabling revokes
before saving configuration. The two files are not an all-or-nothing
transaction: errors are reported and refresh/retry reconciles any published
half. Unrelated drafts survive own-action revision changes. No guest lifetime
lock, execution coordinator or new service thread is involved.

Validation:

- Full Mac workflow locally: **13/13 steps**, 37 Rust tests passed and 82 existing
  hardware tests ignored by the workflow. Unchanged results were reused during
  the final affected-check refresh.
- Seven manager-model regressions passed, including stale/invalid requests,
  all capability choices and interrupted profile publication.
- Native production-archive regressions passed: actual JSON boolean types,
  combo presets/custom values, login actions, permission/draft preservation,
  creation-stage transitions, top alignment and button hit regions at default
  and minimum window widths.
- Disposable running Linux integration passed boolean true/false round-trips,
  permission enable/disable, concurrent creation, stale-write rejection and
  continued guest execution. Existing grant separation checks still pass.
- The existing Mac Dev Debian profile accepted read-only validation patches for
  both boolean states. Its profile bytes and running environment were unchanged.
- English/Chinese resource key parity and syntax passed, with 344 keys each.

Desktop attachment timed out and AppKit reported an inactive desktop session.
OS mouse-event dispatch was therefore explicitly skipped; native hit testing
and control actions ran. Main configuration, permission and settings view
renders were reviewed; inactive-session sheet captures did not provide complete
visual output. A foreground desktop mouse/visual check remains outstanding.

Mac Dev is installed with embedded Vulkan and verified signing. Existing manager
windows should be reopened to load the new code; they were not forcibly closed
because they may contain unsaved drafts. Linux environments and Stable were not
restarted. Runtime/iPad source is unchanged, and roughly 24 GiB remains free.

[Machine-readable evidence](mac-manager-fixes-20261116.json) includes source
hashes and the exact validation/deployment scope. The
[manager architecture](../architecture/mac-manager.md) records the control and
permission contract.
