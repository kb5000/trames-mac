# Trames Mac 1.5.3 acceptance (2026-11-04)

This update repairs large HTTP responses and an intermittent Go/Docker hang.
Runtime: `d9c4a811faf316a51b1a8f33f055247943a18855`.

TCP sends accept a short prefix within the existing 1 MiB staging bound instead
of rejecting an application's larger logical write. Datagram size errors retain
their atomic-message meaning. Stream batches keep the original requested length
and retained calls never replay an accepted prefix.

Signal interruption and task exit explicitly cancel logical futex registrations,
even when an owner checkpoint retains the old Arc. The old ticket can no longer
steal the next wait's wake. Cancellation keeps the original keyed synchronization;
no coordinator, service thread, polling loop or Go preemption workaround is added.

## Validation

Apple ARM64 and native ARM Linux each pass 231 Linux ABI cases (one ignored),
83 network cases, 291 kernel conformance cases (three ignored) and 17 kernel
unit cases. Strict affected Clippy and formatting checks pass. The shared runtime
also passes the physical-iPad FFI target compile; a new device test is not claimed.
All eight pre-build Mac workflow steps pass locally, including strict workspace
checks, product tests and native task-stream acceptance.

The original Python reproducer transfers all 3,777,458 bytes with one `sendall`;
small and chunked controls also pass. Stock Debian Go tools pass 30 GC-stress
starts at four CPUs and 100 at twelve CPUs with normal async preemption, plus
30 native ARM Linux reference starts. An isolated Docker daemon passes eight
start/health/exec/inspect/stop/remove cycles using public Alpine content.
No application data, external model requests or frontend activation is needed.

[Source-bound evidence](https://github.com/kb5000/trames-ipad/blob/main/docs/validation/reader-runtime-20261104.json)
· [Shared contract](../../runtime/docs/architecture/execution-backends.md#stream-prefixes-and-logical-futex-retirement-2026-11-04).

Stable and Dev use separate app identities and existing data registries. The
selected distribution remains locally signed and unnotarized. A running
environment must restart to use the corrected runtime and discard already
stranded waits. Installation preserves environment storage.


## Installed deployment

Both signed app bundles are installed as 1.5.3 with the pinned runtime. The final
stable binary independently passes another eight isolated Docker lifecycle
cycles; final Dev passes the original TCP reproducer and another 30 Go GC starts.
Debian was restarted after confirming the scheduled collection was stuck at
Docker startup and its collection lock was free. The preview container is now
healthy. Its weekly endpoint returns HTTP 200 with all 3,777,458 declared bytes
and valid JSON (one local sample: 62.6 ms). Docker list/inspect return in about
119/111 ms. The collector is inactive and its timer remains active/waiting.
The separate Docker HTTP environment remains healthy and was not restarted.
The frontend candidate has not been activated, and no model/collection API
requests were made by this validation.
