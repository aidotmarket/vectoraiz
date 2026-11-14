Windows C1 feasibility harness (S1944, fold 10)
=============================================

This default-off harness measures the actual parser worker. It does not change
product confinement or qualify Windows/C1. Authority: runbooks origin/main,
`specs/BQ-VECTORAIZ-WEB-SERVER-GATE2.md`, C1 design deltas 2–5 (delta 5 merged
at `59ffe953`), and both Gate 3 reviews of `57e1209e`. Actual ingest/render/OCR,
supported client Windows builds, ordinary/hostile/scanned fixtures and carried
cleanup, resource and confinement obligations remain owed.

Launch and run
-------------

The feature-gated launcher accepts:
`vz-office-windows-launch.exe WORKER MODE REQUEST.json RESULT.json SCRATCH ATTACK_FILE [EXACT_INPUT ...]`.
Use a fresh private evidence root outside the checkout. From PowerShell:

```
$env:OFFICE_EVIDENCE_DIR = Join-Path $env:RUNNER_TEMP "office-c1-local"
cargo build --locked -p vz-parser-worker -p vz-office-windows-harness --features office-feasibility --bin vz-parser-worker --bin vz-office-windows-launch
python scripts/office-windows-feasibility.py --out "$env:OFFICE_EVIDENCE_DIR/run-1" --pdfium C:/exact/pinned/pdfium.dll
```

The runner plans 130 judged rows and three diagnostic groups, including six UDP-listen not-applicable rows. It
creates a sentinel under the user's profile, disposable loopback/LAN TCP/UDP
controls and synthetic TXT/Markdown/HTML/DOCX/XLSX/PDF fixtures. Every judged row has a
separate unconfined control and confined worker. AFD, Winsock registry, socket creation, RPC and DNSAPI rows also record a plain zero-capability AppContainer comparison;
that comparison never substitutes for the LPAC result. Shell rows run the
confined worker before the control, with separate listeners/nonces per API/mode.

Every confined invocation creates a fresh profile, grants its unique package
SID read/execute to the worker and exact loader DLL closure, read to exact
inputs and write only to disposable low-integrity scratch. The OS-managed
AppContainer store and intrinsic Windows runtime ACLs are not host-folder grants.
The worker starts suspended, detached, with only three explicit pipe handles and
a minimal environment. An inheritable host-file attack handle is omitted. The
parent sets and reads back job limits, assigns the job, verifies a single job
member and the requested token, then resumes. Any setup/readback failure refuses
launch; there is no unconfined retry. The unconfined control explicitly inherits
the attack handle and uses a kill-on-close job permitting breakaway.

Successful collection restores temporary DACL grants and deletes the profile.
The runner retries scratch cleanup and records failures without discarding
completed observations. Abrupt launcher termination can bypass profile/DACL
cleanup; kill-on-close still terminates workers. Use disposable Windows runners.

Stage A — verdicts and public evidence
-------------------------------------

Socket creation rows judge only creation. A descriptor is a strict-creation
FAIL. Literal 10107/10093 startup/creation failures can pass only with the
family-specific AFD and registry attribution described below; those codes alone
never establish policy denial. Operation rows run only with a usable socket. If every
tested constructor/AFD route returned no socket, they are unrun with reason
"not exercisable: no socket obtainable by any tested route". Incomplete routes
or an available alternative are identified separately. Operations on an invalid
socket are no longer issued. Operation denial does not erase creation success.
Bind/listen and sendto controls use local reachable endpoints; requested and
actual endpoint scope are separate fields. TCP sendto connects first. UDP listen
is not applicable. Missing routes, families or services cannot establish denial.

A deny row requires the same API as its successful control or a named
prerequisite; the reason records the stage reached. COM initialization refusal
therefore does not claim activation was exercised. Generic errors fail. The
inherited probe enumerates its own handle table, reports presence/object type
and file identity when applicable, and never reads an invalid raw handle.
STATUS_INVALID_HANDLE exits are classified distinctly.

Commit rows require kernel allocation refusal and held allocation bytes at or
below the configured ceiling. Reported job/process peaks remain separate fields;
a refused request can inflate them. Child quota error 1816 is attributed only
with active-process-limit readback. Scratch operations must succeed; parser
rows require all serialized fields equal, content oracles and page/sheet checks.
These synthetic extraction controls do not establish rendering/OCR coverage.

Stage B — delta 5 memory rows
----------------------------

Every confined row uses JOB_OBJECT_LIMIT_WORKINGSET, minimum 64 MiB, maximum
1.5 GiB, read back before resume. Ordinary process/job private commit ceilings
are also both 1.5 GiB. Process-only and job-only diagnostic rows loosen the other
commit ceiling to 3 GiB while retaining the working-set cap, active-process one
and no-breakaway restrictions. Actual PID/image membership is recorded.

Private VirtualAlloc/malloc/many-small/2 GiB burst allocations retain memory
until refusal/target, with the sampler disabled. Pagefile-backed SEC_COMMIT and
SEC_RESERVE sections map and touch every page. Scratch-file-backed rows resize a
file to 2 GiB, map it and touch every page, then require scratch cleanup. Parent
kernel peak working set must remain at or below cap + 4,096 bytes. Completion by
paging is the documented residual, not a failure or a kernel-refusal claim.
Worker snapshots and parent samples retain system commit and physical
available/total before/after in the same row, alongside private/resident counters.

Self-ws rows attempt SetProcessWorkingSetSizeEx with a larger maximum and with
QUOTA_LIMITS_HARDWS_MAX_DISABLE, record return codes and process/effective-job
readbacks, then touch a fresh section after each attempt in the same process.
The hard-ws diagnostic adds a per-process hard cap to the job baseline; it does
not establish that the per-process cap alone resists mutation. Alter-job tries
raising commit ceilings and clearing the working-set flag if it can open the job.
The notification experiment also retains completion-port messages and errors;
absence of a notification does not establish absence of memory use.

The separate section-sampler row polls private commit, working set and run-window
system commit growth every 50 ms and terminates the job over budget. Missing
sampler data causes termination and FAIL. It records overshoot, detection/kill
latency and parent-loop gaps. Responsiveness means this launcher's loop stays
within one second, not a product UI claim. A kill cannot pass kernel-refusal rows.
System-wide commit deltas include concurrent host activity: isolation to one
worker narrows attribution but does not prove causality on a shared host. Extra
section/page-file use can still slow or stall the PC before best-effort termination.

Stage C — attribution and broker evidence
----------------------------------------

RegOpenKeyExW reads HKLM SYSTEM/CurrentControlSet/Services/WinSock2/Parameters and
its Protocol_Catalog9 and NameSpace_Catalog5 subkeys. WSCEnumProtocols records
sizing/enumeration statuses; these and the plain-AppContainer comparison retain
WSAStartup results. Denied prerequisites are evidence for investigation, not
proof of all Winsock or network confinement.

Direct NtCreateFile opens the AFD endpoint using an AfdOpenPacketXX EA for
IPv4/IPv6 × TCP/UDP, without ws2_32 initialization. The private packet layout
follows [System Informer's phnt definitions](https://github.com/winsiderss/systeminformer/blob/4d718e99ac16afa37b3c17512d126cd6d0a6af63/phnt/include/ntafd.h).
A succeeding same-build control handle is required. Confined STATUS_ACCESS_DENIED
without a handle is an explained narrow kernel denial; an obtained handle fails
strict creation. Other statuses remain unproven. The private ABI is not assumed
stable across supported client builds; successful controls validate each run.

Three separate shell rows exercise ShellExecuteW and ShellExecuteExW with
SEE_MASK_NOASYNC, once with FLAG_NO_UI and once without. They open a unique
http loopback URL against the harness listener, run confined before control and
retain nonce-correlated GET observations. A control must report API success and
receive GET. Confined API success or a GET fails. No GET alone cannot pass:
explicit broker denial is still required. Last error after TRUE is ignored.
Process snapshots are supplementary and cannot rule out short-lived dispatch
or existing-handler reuse. A UI prompt can time out and fail the invocation.

The RPC row binds ncalrpc:[epmapper], sets a bounded call timeout and performs an
endpoint-map inquiry, recording literal statuses and reached stages. Its call is
[RpcMgmtEpEltInqNextW](https://learn.microsoft.com/en-us/windows/win32/api/rpcdce/nf-rpcdce-rpcmgmtepeltinqnextw),
with a succeeding control required. Endpoint unavailability is not access denial.

CI, traffic and validation
--------------------------

The workflow runs windows-2022 only on the feasibility branch or manual dispatch.
All actions use immutable SHAs, checkout credentials are not persisted and
permissions are read-only. It follows native CI setup with pinned Rust/PDFium,
without OCR/model acquisition. It initializes all rows before building and
retains the latest partial manifest on failure. Every fail/unrun row yields a
nonzero suite result. This does not impose a green qualification expectation.

Controls create live traffic: TCP to 1.1.1.1:443, UDP to port 53, DNS lookup of
example.com and local TCP/UDP endpoints. Shell controls open the default browser
three times against nonce-tagged loopback HTTP listeners; they no longer request
example.com URLs. Browser background activity is outside listener attribution.

Requests, URLs, fixture bytes, raw responses and complete subprocess/HTTP logs
remain under the private evidence root. Only manifest.json is uploaded: parser
responses become hashes and bounded error excerpts, with paths redacted. Complete
streams spool privately even beyond the 32 MiB in-memory capture budget; overflow
fails the invocation. Bounded setup/build/token/suite log tails appear in CI logs.
Manifest validation scans decoded strings and keys recursively for drive paths,
IP literals, UNC paths, home/user paths and user-name patterns, and validates
completeness/counts. The backslash pattern deliberately remains fail-closed: even
a harmless reason containing word-backslash-word is rejected. Use plain prose
or slash-separated logical device labels; native paths stay private. Artifact upload requires that
validation step to succeed; a leak or malformed manifest fails closed.

Portable verification: `python scripts/office-windows-feasibility.py --self-test`.
Windows-target check/strict clippy of the standalone harness work on the Mac;
Windows linking/runtime and the actual worker's native C++ dependencies require
Windows SDK/MSVC. The operator reads CI outcomes; no CI success is asserted here.

Historical observations (not qualification): runs 43477570090 and 43484043382
showed sections escaping job private-commit accounting and completing under a
working-set cap; run 43491499188 at the Gate 3 base reported 27 pass / 71 fail /
4 unrun / 6 not-applicable. Fold 7 changes evaluation and adds evidence probes;
these prior counts do not predict the new result.

Fold 8 preserves the one-page tolerance for every section row, including the
file-backed view. Failure reasons print the parent-read peak and ceiling. In run
43514641441 the process-only file view peak was 1,610,620,928 bytes (cap + 8,192),
so it remains a failure under the cap + 4,096 rule; the peak was present.

The AFD EA uses an explicitly eight-byte-aligned C structure, following the
[phnt layout](https://github.com/winsiderss/phnt/blob/master/ntafd.h). Its packet value
is 28 bytes (six 32-bit fields, WCHAR[1], tail padding), starts at byte 24, and
EaValueLength and the submitted buffer length derive from that layout. Padding
is zeroed. A successful same-build control returning a handle remains required.

Catalog, RPC and DNS retain numeric return/error codes and a hexadecimal literal
status. Explicit access denial (5, 0x80070005, 0xC0000022, or Winsock WSAEACCES)
is accepted only after a successful control and the existing API/prerequisite
check. Other failures print their codes and stay FAIL. The prior run's catalog
8, RPC 14 and DNS 10093 are not access denials. Socket creation and shell row
classification are unchanged. These harness changes make no qualification claim.

Fold 9 keeps literal socket creation and policy attribution separate: constructor
rows require a matching passing AFD control/denial and all three catalog registry
denials. Evidence applies only to the recorded OS build and zero-capability LPAC.
The malformed-family comparison is diagnostic only. It establishes neither
internal check ordering nor denial for an untested valid family, and does not
substitute for successful per-family controls. Catalog enumeration is
a diagnostic on the protocol-registry row. RPC runtime errors remain literal
failures; ALPC epmapper and DNSAPI have separate controls.

All three section modes have sampler experiments. Pagefile-backed modes trigger
on working set or system commit growth at or above the cap. The file-backed
experiment uses a 1,207,959,552-byte role intervention budget (three quarters of
the cap) and also samples logical scratch backing-file size. File sizing can
trigger before mapping/touch: this is a disk-budget signal, not measured resident
memory or proof of a hard total-memory ceiling. Each row records its threshold,
cap, samples and firing signal; a missed trigger stays FAIL. System growth
includes unrelated work.
Detection time is captured before termination; kill completion is separate.
Responsiveness measures only a launcher loop proxy. Self-mutation rows exercise
the combined job and process caps, not an independent per-process defense.

Cleanup limitation: if Python kills the launcher at its 70-second deadline, Rust
destructors cannot run: temporary package ACEs and the AppContainer profile can
remain. Preserve private evidence and restore original ACLs/remove that profile
before reusing a persistent runner. Reverse grant unwinding covers ordinary early
errors, but does not solve forced launcher termination. No qualification claimed.

Fold 10 diagnostics and evaluator corrections
--------------------------------------------

AF_HYPERV direct opens use protocol 1, matching Winsock; a control without a
handle keeps the row unrun. `afd-unix-tcp` retains its historical id but records
transport `stream` and an AF_UNIX stream reason. The malformed-family comparison
is attached to each AFD row and excluded from counts. No malformed result can
qualify an untested valid family. DNSAPI checks its control first, records a
plain-AppContainer comparison, and keeps 9852 (`DNS_ERROR_NO_DNS_SERVERS`) FAIL.
Self-raise/disable accepts explicit access/privilege refusal with unchanged
before/after readbacks and valid section evidence, or successful calls with no
effect; reasons distinguish each attempt. Changed limits and unexplained errors
fail. These are combined job/process-cap experiments.

The reachability diagnostic enumerates at most 200 entries under RPC Control
and connects to ALPC port candidates, then opens Device/Afd, Device/Tcp,
Device/Udp, Device/Mup and NamedPipe/{epmapper,lsarpc,samr,srvsvc}. The same
control-discovered candidate list is attempted from the control, plain
AppContainer and LPAC. The manifest table has those three literal-outcome
columns, including unattempted/deadline outcomes. ALPC names are SHA-256 identities
correlated to native names in private requests/results; device labels use slashes.
Enumeration/query statuses and truncation/budget evidence are retained. Limits
are six seconds per mode (18 seconds aggregate attempt budget, excluding launch
and cleanup), 20 ms per ALPC connect, and 200 candidate ports plus eight devices.
A successful connection is closed; no ALPC message or device I/O is sent. This
inventory neither enumerates every Windows IPC route nor changes `rpc-alpc`'s
strict-denial evaluator. Reachable epmapper continues to fail that judged row.

AF_UNIX outcome diagnostics create an unconfined host listener at a non-granted
path and try confined connect to it, bind outside scratch, bind inside granted
scratch, abstract bind and connect to a separate host abstract listener if
supported. Host startup/bind/listen statuses are public without addresses.
The direct AFD route uses TLI bind/connect layouts from
[phnt/ntafd.h](https://github.com/winsiderss/phnt/blob/master/ntafd.h), with
same-route control outcomes required to interpret the private ABI. Connect first
binds its local endpoint inside scratch. Pending operations wait at most one
second before cancellation; failed cancellation aborts the worker rather than
releasing buffers still owned by the kernel. A Winsock fallback is attempted if
no AFD handle is obtained. All literal statuses, prerequisites and route choices
remain diagnostic; unsupported namespaces or unsuccessful controls establish no
policy denial. No socket payload is sent. `afd-unix-tcp`, `rpc-alpc` and shell
judgment rules are unchanged. No qualification is claimed.
