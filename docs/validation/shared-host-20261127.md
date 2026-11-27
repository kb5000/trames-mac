# Shared host transports — 2026-11-27

Mac now imports the Runtime exec/console servers through two small adapters.
The independent CLI repository retains clients and argv/terminal/signal policy.
Wire identity and framing limits are checked at compile time. The Runtime
servers use the original foreground wait, VFS pipes/PTYs and task admission;
no coordinator or service thread was added.

[Acceptance](shared-host-20261127.json): 13 real-HVF groups passed on the final
paired CLI, including 8 MiB full-duplex binary streams, separate stderr/exit
status, literal argv/environment/cwd, concurrent exec, private PTYs, signal
isolation, independent shells, persistence and lifecycle. Product/HVF units:
32 passed; 82 existing hardware/pressure cases remain explicitly ignored.
The five moved server tests pass in Runtime on Apple and Linux, together with
44 distro tests. Final provenance-guard coverage and full workspace results
are also covered by the paired Runtime CI.

Runtime pin: `3f5d4f53bb3dae0449b03cd0a8a0fe1ea8562cc8`. Its history preserves
the separately validated Linux feature commits. The Mac pin metadata now
matches the actual submodule; obsolete extraction-only metadata was removed.
Formatting, boundary validation, locked Clippy and license fingerprints pass.
No user environment, app installation, iPad source or independent CLI pin was
changed by this adaptation.
