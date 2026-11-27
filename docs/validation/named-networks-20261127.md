# Shared named network registry — 2026-11-27

Mac now delegates cold named-network metadata, stable UUID membership and host-grant intersection to `trames-distro::networks::Networks`. Its existing manager listing, Runtime fabric, event waits and live controls remain connected. Host-authorized wildcard publications bind the actual host address. The standard managed resolver address also works through an attached fabric.

The [record](named-networks-20261127/summary.json) identifies Runtime `88be4442dec1f9a8095796c422016b1b3f030e57`, exact source hashes and the signed real-HVF runner. Workspace all-target/all-feature Clippy and build passed. [22 host commands](named-networks-20261127/commands.json) passed: actual native host socket space, cross-process bridge isolation, guest port reuse, live host publication and original-resource mount control.

A source-isolated copy used committed `session.rs` while retaining its owner's unrelated in-progress memory-pressure changes in the original checkout. This acceptance does not cover that concurrent work. The existing fixture was updated to the current `trames debug rpc` command and current mount configuration keys; failed stale-fixture runs were not counted.

The paired Linux product runs its own KVM SMCCC/MMIO × one/four-CPU network matrix. No Linux GUI is introduced.

The clean paired Mac commit also passed [formal CI](https://github.com/project-trames/trames-mac/actions/runs/45939741723), including workspace all-feature checks/tests, native integration checks and both Dev/Stable product builds. [Recorded steps](named-networks-20261127/ci.json) bind the result to `761dd7f53e7ed215b6d94b811717eb33fcc45f1d`. This separately verifies the actual published clean source tree.
