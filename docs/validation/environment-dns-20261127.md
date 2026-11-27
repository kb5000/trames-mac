# Environment-name DNS acceptance — 2026-11-27

Runtime `8bd058556ffb35be8e127afbaf7c1c26dae998d9` adds endpoint-owned DNS
leases. This Mac adapter passes the registered environment name to the same
Runtime fabric used by Linux. No new network thread or CPU-loop check is added.

Actual signed Mac HVF passed **66 commands**, one/four CPUs: original guest
`getaddrinfo` A/AAAA queries for names, case folding and qualified names;
cross-network isolation; stop invalidation; rename with retained address; and
restart. The ARM64 guest rootfs is the existing small Linux acceptance input,
with hashes retained; it is not a newly downloaded image.

[Evidence](environment-dns-20261127/results.json) binds the signed runner
`d5e46e2fda3347d120fb71def1e9782750d7b51f1616fd92a9524b6dc50f50c1`
to its [source/build receipt](environment-dns-20261127/build-receipt.json).
The receipt command includes build, ad-hoc Hypervisor signing, and signature
verification. [Commands](environment-dns-20261127/operations.json), the frozen
harness, rootfs input hashes, build helper, all-feature unit checks and strict
all-target/all-feature Clippy are retained in the evidence directory. GUI and
unrelated CPU feature tests reuse their unchanged results.

Within a named network, `db.trames` and `db.env.trames` resolve only while its
endpoint is alive. Existing `envID.trames` aliases remain. `NAME.env.trames`
also covers reserved labels such as `gateway` or `env2`. Names are case
insensitive; active collisions in one network reject startup. Local answers
have TTL zero. Rename requires stopping; clones/restores have separate UUIDs
and names. Host clients continue using explicit native publications.

The reproducible entry point is `scripts/validate-environment-dns.py` with
`--runner`, `--receipt`, `--rootfs` and `--output`. Linux's separate native
systemd user-service work remains in the Linux product.

Full [Runtime Linux/macOS CI](https://github.com/project-trames/trames-runtime/actions/runs/45966807752) and [Mac CI](https://github.com/project-trames/trames-mac/actions/runs/45967467914) passed, including Dev/Stable builds. The [CI ledger](environment-dns-20261127/ci.json) and [source closure](environment-dns-20261127/source-closure.json) retain tested commit identities and prove the final product inputs match the signed HVF receipt. Documentation-only closure reuses these passed results.
