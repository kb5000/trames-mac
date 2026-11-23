# Manager sidebar clipping — 2026-11-23

The environment table could start wider than its visible scroll viewport. At
280 pt sidebar width, the source-list table initially measured 282 pt while the
viewport measured 256 pt. The source-list style adds horizontal row insets to
the configured 250 pt column. The selected row and trailing status indicator
were therefore clipped until a later sidebar resize corrected the table size.

A sidebar-specific scroll view now fits the table document to its actual content
width after Auto Layout resolves the viewport, and fits the last table column.
The sidebar retains native source-list selection and its existing column insets.

Validation:

- Reproduced the initial 282/256 pt mismatch using the existing native manager
  fixture before changing production code.
- Native regression assertions passed on first presentation and repeated sidebar
  widths of 280, 380, 250, 340 and 280 pt. The table and selected row remain within
  the viewport; the trailing status indicator keeps at least 6 pt of clearance.
- `tests/native/task-stream.sh` passed: task streaming, manager progress/actions,
  manager layout and native input/clipboard checks. OS mouse-event dispatch was
  unavailable for the fixture, so the existing native hit-target/action fallback
  ran; the layout and geometry assertions were not skipped.
- Dev and Stable Release builds and code-signature verification passed.
  Both applications were installed to the existing per-user Applications paths,
  and their managers were restarted. Existing environment service processes were
  retained. UI automation could not read the application's accessibility tree;
  visual acceptance relies on the native fixture geometry rather than a claimed
  live screenshot comparison.

Changes are limited to the native manager layout and its regression test.
Runtime source and the repository's Runtime pin are unchanged. Test screenshots
and logs are under `/private/tmp/trames-sidebar-20261123/`; build/install logs are
`target/sidebar-{build,install,stable-build,stable-install}-20261123.log`.


## Host Interop follow-up

The owner also requested removing the per-option confirmation dialog from Host
Interop. Selecting Allowed or Not Allowed now saves that combined environment
permission immediately. Existing revision checks and inline operation state are
retained, and changing a permission preserves unrelated configuration drafts.

The native manager regression toggles Clipboard and Trames CLI on and off through
their real target/actions, checks that no confirmation method/sheet/modal window
is invoked, and verifies the saved command and retained draft. It passed along
with the existing manager layout/action assertions. The scrollbar assertion now
checks the two app-owned scroll views and accepts the system's preferred legacy
style, which AppKit can apply during the test session. Earlier assertions that
required overlay style regardless of the system preference failed before the
permission section and were corrected without changing production scroll code.
