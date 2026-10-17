# S1806 owned Unix lifecycle fixture — source only

No native observation has been made. R289/R328/R339 and the Full16 audit remain
open. This is a standalone stdlib operator driver, not a pytest test. It must
not be imported through the application or test conftest. Production Go,
TestMain/allowNativeExecution, installers, and security paths are unchanged.

Contract read from the preserved reviewed checkout at
`1d1766fa83567304e5894d54e1cc0b50573f56a3`: `host-updater/process.go`,
`os_unix.go`, `main.go`, and `storage.go`. The assigned publication base
`985cf1b73e35777f7e7db2c1a9d4d81de4b7ae76` has no host-updater tree. No Go files
were copied into this branch. A future operator must select an existing trusted
built owner implementing that contract. The fixture records its SHA-256; it
cannot establish the source provenance of an arbitrary supplied binary.

## Future authorized use only

Do not execute these commands under the current source-only authorization.
Without `--native`, the driver describes/refuses native execution with exit 2.
It requires Linux `/proc`, Python 3.11+ with pidfd APIs, and existing `rtk`.
Official RTK v0.34.3 source (`src/main.rs`, `Commands::Proxy`) spawns a child,
streams output, and waits; it does not exec transparently. The driver pins the
retained wrapper, then discovers only its sole actual child through that
wrapper's kernel child lists. Darwin and other proxy topologies are unsupported;
no numeric PID/group signals are substituted. This topology is source-derived,
not natively verified against the installed binary.

Source: https://github.com/rtk-ai/rtk/blob/v0.34.3/src/main.rs#L1932-L2035
(`src/core/utils.rs::resolved_command` constructs a direct `Command`).

After separate explicit native authorization, use an existing owner binary and
an existing, owned, private (0700 or equivalent), disposable output directory:

```text
rtk proxy python3 tests/update_owner/owned_process_fixture.py --native --owner-binary /absolute/path/to/existing/vectoraiz-owner --output-root /absolute/path/to/private-disposable-directory
```

The supplied directory is never removed or pruned. A unique private
`TemporaryDirectory` is deliberately retained beneath it, including logs,
synthetic records, per-case `outcome.json`, and `receipt.json` on failure as well
as completion. Only this new directory receives fixture writes. The printed
receipt location is local; receipts omit host environment, binary location,
credentials, and customer data. Binary and driver hashes are recorded, with
binary source provenance explicitly UNKNOWN. Do not replace the binary during
a run. The input binary is an explicitly trusted executable, not sandboxed code.

## What the native mode would observe

Each independent case starts `owner command ROOT ID` against a fresh synthetic
`child.json` with an empty environment/input, the same original absolute Unix
deadline, and a benign Python command under that case directory. That command
retains a Popen for one bounded idle grandchild. The command/helper/watchdog
argv and groups must match the actual reviewed product contract. All operator
Popen argv begin `rtk proxy`; the product's own helper/watchdog and fixture's
internal child calls use ordinary direct argv without shell interpolation.

* **Completion:** release the benign child through a private file; observe real
  child/grandchild/helper/watchdog exit, durable `Done`/empty `Code`, and release
  of the actual `child.lock` via nonblocking flock. This does not claim to
  exercise `Processes.Run`'s durable-result polling or handoff serialization.
* **Helper death:** after all processes are independently pinned and alive,
  SIGKILL only the retained helper pidfd. Observe actual watchdog, command child,
  and grandchild termination within two seconds and before the original deadline.
  A missing durable result is recorded honestly, without reconciliation/replay.
* **Original deadline:** send no intervention signal during observation. Sample
  actual termination against the unchanged original absolute deadline, retaining
  the predeadline-live control. Each role records its first dead sample, latency
  relative to that deadline, and measurement interval from the start of its last
  live sample to the end of its first dead sample. OBSERVED means all four owned
  targets were observed exited within the existing `deadline + 0.25` polling
  allowance, with the sentinel alive. This small existing window accommodates
  timer delivery and polling; it grants no new command budget and asserts no
  exact-boundary termination. Samples at +5ms or +200ms qualify this bounded
  claim; beyond +250ms, missing targets, sentinel death, or unknown ownership
  remain UNKNOWN. The exact deadline instant remains unproved.

An independently launched, pinned sentinel in its own session/group must be
alive at the observation point in every case. It is killed only during cleanup.
Linux pidfd readiness means process exit (including a zombie), not disappearance
from `/proc`. Direct Popen children are reaped; orphan reaping is not asserted.

The driver holds the real child lock before helper launch, establishes a pinned
helper/sentinel cleanup guardian before unlocking, and then replaces it with a
guardian holding all role and retained-wrapper pidfds before any experimental
signal. Guardians are external processes with a 30-second cap; EOF or timeout kills only pinned owned
processes. Live ancestry, exact argv, UID, process group, stable kernel start
identity and pidfd liveness are verified before accepting ownership. The helper
PID must match the pinned wrapper child and the product record. A minimal
Python bootstrap makes that child a private group leader before exec of the
helper, preserving the product watchdog's helper-PID/group contract. The wrapper
remains in its own launch group. Sentinel ownership likewise follows its pinned
wrapper's sole child. Wrapper identity and exact ancestry are checked again after
child pinning. Kernel child-list reads are scoped to verified owned processes,
never a host process scan. No persisted numeric identity is used for signaling. No group kill, pkill, process
prune, shell command, or retry of a case is performed. Short polling for initial
exec/readiness does not relaunch anything or manufacture an outcome.

Setup failures stop the run. Already verified pidfds are cleaned; an unverified
Popen is never signaled. Such a failure is UNKNOWN and may leave the bounded
25-second sentinel/fixture or deadline-bound helper to exit on its own. Do not
interpret incomplete setup as cleanup proof. Unexpected descendants/ownership,
sentinel death, or missing completion also stop subsequent cases. The fixture
has only a handful of bounded idle children and never forks recursively.

## Deliberate limits

There is no safe CLI entry into `Processes.Run` polling independent of full
owner state. Public `reconcile` requires registration/integration and eventually
Docker revalidation; `launch` can start a platform supervisor. Neither is called.
Polling/launching-parent exit, real Run/Reconcile handoff, and recovery outcome
are NOT_RUN. A fabricated registration, status, supervisor marker, or command
publication path is not used to force those observations. The direct command
record is the existing internal CLI contract, not a new public publication API.

No daemon/socket, listener/port, network, database, application, auth/provider,
real Docker, supervisor/install/service, shared resource, or customer data is
used. There is no reboot/platform-supervisor/full-updater/adoption success claim.
Exit zero would mean only the listed observations met the fixture's strict
conditions; it would not close the audit. UNKNOWN/NOT_RUN aspects require further
explicitly authorized work. Native execution, builds, imports and source tests
were not part of this source-only delivery; validation is AST parsing and Git
diff/source inspection only.
