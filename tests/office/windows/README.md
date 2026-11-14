Windows C1 feasibility harness (S1944, fold 11)
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

The runner plans 142 judged rows and four diagnostic groups, including six UDP-listen not-applicable rows. It
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

By-hand run on a Windows 11 PC
-----------------------------

Allow 30–45 minutes including download, checksum verification and inspection;
selected probes normally take about 8–15 minutes. This is an **unqualified
feasibility measurement**; omitted rows keep the whole manifest incomplete.

Prerequisites: an x64 Windows 11 **client** (ProductType 1, build 22000 or newer),
an interactive signed-in desktop with a working default HTTP browser, Python
3.12 x64, and an **elevated PowerShell** (Start → PowerShell → Run as
administrator). Microsoft Office, Visual Studio, Rust, models and OCR assets
are not needed. The kit includes the pinned PDFium; CI checks the native loader
closure and builds with the static MSVC runtime. Only the kit download requires
network access; the selected shell controls use a disposable loopback HTTP
listener, and the file control opens a synthetic `.cmd` file, not an Office file.
Do not run another harness or change audit settings during the measurement.

1. Open the repository's Actions page, select the **Office C1 Windows feasibility
   (unqualified)** run for the intended commit, and download
   `office-windows-run-kit-<run_id>` from its Artifacts. Retention is **3 days**.
   The other artifact, `office-c1-windows-feasibility-<run_id>-<attempt>`, is the
   Server runner manifest; it is not the client measurement. Extract the ZIP to
   a fresh folder, for example `$HOME\Downloads\office-windows-run-kit`.
2. In elevated PowerShell, set the extracted folder and verify the **exact** file
   set and all five hashes before running any kit code:

   ```powershell
   $ErrorActionPreference = 'Stop'
   $kit = (Resolve-Path "$HOME\Downloads\office-windows-run-kit").Path
   $expected = @('vz-parser-worker.exe', 'vz-office-windows-launch.exe', 'pdfium.dll', 'scripts/office-windows-feasibility.py', 'scripts/office-windows-audit.ps1')
   $actual = @(Get-ChildItem $kit -Recurse -File | ForEach-Object { $_.FullName.Substring($kit.Length + 1).Replace('\','/') })
   if (Compare-Object ($expected + 'SHA256SUMS' | Sort-Object) ($actual | Sort-Object)) { throw 'Unexpected kit files' }
   $sums = @(Get-Content "$kit/SHA256SUMS")
   if ($sums.Count -ne 5) { throw 'Expected five checksums' }
   $seen = @()
   foreach ($line in $sums) {
     if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw 'Invalid checksum line' }
     $hash = $Matches[1]; $file = $Matches[2]
     if ($file -notin $expected -or $file -in $seen) { throw 'Unexpected checksum path' }
     $seen += $file
     if ((Get-FileHash (Join-Path $kit $file) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash) { throw "Hash mismatch: $file" }
   }
   python --version  # Must be Python 3.12.x
   ```

3. Run this exact command from that PowerShell. Keep the desktop signed in and
   allow the control browser windows to open. The evidence folder must be fresh
   and outside the extracted kit:

   ```powershell
   $out = Join-Path $env:LOCALAPPDATA ("office-c1-byhand-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
   python "$kit/scripts/office-windows-feasibility.py" --kit-dir "$kit" --rows client-outcomes --out "$out"
   python "$kit/scripts/office-windows-feasibility.py" --validate-manifest "$out/manifest.json"
   ```

The redacted result is `$out/manifest.json`; share only that file. Raw logs,
Security event XML, paths, nonces and the saved audit settings in `$out` stay
private. The selected group contains `shell`, `shell-ex-no-ui`, `shell-ex-ui`,
`shell-file`, `unix-2a` through `unix-2d`, the `af-unix-outcomes` diagnostic, UNIX
socket/AFD creation and malformed-family/registry controls, and `scratch-control`.
Each probe includes its ordinary control and, where planned, plain AppContainer
comparison. The suspended launch diagnostic still runs. Every other planned row
is `unrun` with reason `not selected in by-hand run`. A nonzero suite exit is
expected for this partial run; successful manifest validation confirms structure
and redaction, not passing outcomes or qualification. Individual missing controls
remain unrun. Shell rows report edition/build and the client-SKU check; a Server
build can never satisfy the Windows 11 client requirement. Without elevation,
shell rows and the audit-dependent UNIX 2c row say why they are unrun.

Cleanup and undo: on normal completion or Ctrl+C, the runner closes its local
HTTP/AF_UNIX listeners, removes scratch/sentinel directories, and restores the
previous audit policy and command-line auditing registry value (including prior
absence). It adds **no firewall rule**. The launcher restores its temporary file
ACL grants and removes each temporary AppContainer profile. Check
`audit_restoration.success` and any `cleanup_error` in the manifest before
removing private evidence. Browser tabs opened by the positive controls can be
closed by hand.

If the terminal/process was forcibly killed, run the following in elevated
PowerShell **before deleting evidence** (set `$kit` and `$out` to the same folders
if reopening the terminal):

```powershell
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$kit/scripts/office-windows-audit.ps1" -Mode restore -State "$out/audit-before.clixml"
```

This restores the saved audit-policy backup and the prior registry value. Use it
only if `audit-before.clixml` exists; a run refused for lack of elevation made no
audit changes. A forced kill can bypass native ACL/profile cleanup; retain the
private launcher/cleanup logs and resolve their recorded leftover resources
before deleting them. Normal cleanup failures are recorded rather than hidden.
After inspection, delete the extracted kit and `$out` to remove downloads,
private logs and audit backups. There is no installed service or persistent
listener to uninstall. This run makes no Windows/C1 qualification claim and does
not change any delta 6 judgment rule.

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
The assembly self-test covers the alternative socket route's refusal, reachability,
failed control, failed launch and missing-calibration branches; run-wide listener
completion, early exit, connection and capture failure; and forged mapper reply
flags. Post-call 4688 observation durations use a monotonic measurement around the
wait and capture, rather than a literal ten seconds.
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

The reachability diagnostic seeds epmapper and four service ports before discovery,
then enumerates RPC Control until STATUS_NO_MORE_ENTRIES, a 5,000-entry cap or a
six-second mode deadline. It deduplicates candidates and appends eight device
opens (including Device/Afd/Endpoint). Other device entries are bare device object
opens. These do not substitute for the judged AFD endpoint creation rows.
The table publishes readable slash-form service identities and indexed
random-per-boot classes, with literal outcomes for control, plain AppContainer
and LPAC. The count summary states enumeration complete or truncated/query failed
and whether attempts completed. Each mode has a six-second attempt budget and
ALPC connections use a 20 ms timeout. No message or device I/O is sent. This
inventory does not enumerate every Windows IPC route or change rpc-alpc's strict
denial evaluator.

AF_UNIX outcome diagnostics create an unconfined host listener at a non-granted
path and try confined connect to it, bind outside scratch, bind inside granted
scratch, abstract bind and connect to a separate host abstract listener if
supported. Host startup/bind/listen statuses are public without addresses.
The direct AFD route uses TLI bind/connect layouts from
[phnt/ntafd.h](https://github.com/winsiderss/phnt/blob/master/ntafd.h), with
same-route control outcomes required to interpret the private ABI. Connect first
binds its local endpoint inside scratch. Pending operations wait at most one
second before cancellation; failed cancellation aborts the worker rather than
releasing buffers still owned by the kernel. Control and plain AppContainer use Winsock socket/bind/connect. The control also
calibrates eight fresh-handle direct-bind variants per namespace: full/short
address, Win32/NT path, absent/address-sized output buffer. LPAC uses only a
variant whose bind succeeded in that control namespace. Winsock fallback also
runs after direct bind/operation failure, including successful endpoint creation.
Missing calibration reports `inconclusive: direct route not calibrated`; no
successful control operation reports `inconclusive: control did not succeed`. All literal statuses, prerequisites and route choices
remain diagnostic; unsupported namespaces or unsuccessful controls establish no
policy denial. No socket payload is sent. `afd-unix-tcp`, `rpc-alpc` and shell
judgment rules are unchanged. No qualification is claimed.

Fold 11 adds a diagnostic AFD stream family sweep, families 0 through 40 with
protocol 0, plus protocol 1 for family 34, in all three token modes. Literal
statuses remain diagnostic only. Diagnostic completion requires a successful
control operation; launcher completion alone does not suffice.

All working-set rows record `peak_excess_bytes` (kernel peak minus cap; null if
unavailable). The three memory-*-mapping-file rows and section-job-ws-self-ws
run three control/confined pairs in the same job, retain every repetition and
fail if any repetition fails the existing cap + 4,096-byte rule. The row excess
is the maximum across the three repetitions. This does not relax tolerance or
relabel prior FAIL evidence. Scratch-file sampling is gated to sampler rows;
the file sampler reason explicitly identifies the logical budget and firing
signal. No qualification is claimed.
Fold 13 / delta 6 (current evaluator)
-----------------------------------

This supersedes the historical fold descriptions elsewhere in this document. This is an unqualified
server harness. It does not establish the delta 6 exit evidence on client builds.

* The six non-IP constructor rows and the mapper ALPC connection row are
  observation-only `not-applicable` rows, excluded from pass counts. Non-IP
  constructor results record endpoint creation without claiming creation denial. Separate 2a–2e outcome rows judge reachability. Filesystem rows require
  a successful control operation and a bind inside granted scratch in LPAC
  through the same route. Plain AppContainer cannot qualify them.
  Calibration now also tries 128/256-byte output buffers. Abstract local bind uses
  its own namespace. Status is published per namespace, including successful
  abstract binds even when filesystem calibration fails. Each namespace distinguishes
  a failed control from an uncalibrated confined route. An EINVAL control or
  failed local bind never establishes platform unavailability for 2d.
  Fold 16 retains every direct calibration status and adds a second route using
  `WSASocketW` with the successful unconfined control's exact `SO_PROTOCOL_INFOW`
  record. This adds no capability or catalog ACL grant. Both control and LPAC must
  bind inside scratch through this route before its outside bind/connect outcomes
  can qualify 2a/2b. Failed construction or an unattempted operation stays unrun.
  Scratch create/read/rename/delete status, path byte length and the existing
  inherited package-SID full-access grant (including FILE_ADD_FILE) are recorded.
  Run 43610803971 failed filesystem AFD binds in the unconfined control already:
  full packets returned `0xC000000D`; shortened packets also returned `0xC0000141`.
  Those results do not establish an LPAC ACL failure or platform unavailability.
  The alternative route is an experiment, not a calibrated-route claim.
* A dedicated zero-capability LPAC worker attempts abstract AFD bind/listen before
  the first runtime probe, then continuously observes its listener with bounded
  waits until the parent requests stop after the final probe. The launcher grants
  this operation a 1,530-second deadline; the listener's own bound is 1,500 seconds.
  The parent derives coverage from readiness, liveness after the probes, measured
  duration, successful launch/collection and orderly stop. Row 2c checks the
  listener's own bind and listen success and records its observed namespace.
  It remains `unrun`: the observation is an abstract-namespace listener only;
  no connect route to it is established on this build (see 2d); the listen-wait
  detector has no positive control; the filesystem listener is unmeasured; and
  there is no vectorAIz process on the runner. Listener and run-wide process
  capture evidence remain in the manifest. A future PASS needs a control-mode
  connect to the same listener construction that completes the wait. An observed
  connection still fails the row. These limitations do not change the delta 6
  judgement rule. Rows 2a/2b and filesystem calibration remain open per the
  delta 6 Open item: LPAC WSAStartup fails with 10107 before the explicit-protocol
  constructor fails with 10093, and unconfined direct filesystem binds fail with
  `0xC000000D` / `0xC0000141`. None establishes filesystem policy denial or
  platform unavailability.
  Hyper-V/WSL/Sandbox feature and VM
  inventory records why 2e is NOT EXERCISABLE on an unequipped host. On an equipped
  host, a calibrated utility-VM service is still required; absence of that control
  stays UNMEASURED, not a refusal. The current Windows runner has Hyper-V,
  VirtualMachinePlatform and WSL enabled: 2e is UNMEASURED and cannot support a
  limitation about PCs running Hyper-V being untested. The row retains host evidence.
* The scoped inventory is now a judged row, requiring complete enumeration and
  attempts. Its reviewed list is RPC Control/epmapper (mapper outcome separately
  owed) and Device/Afd/Endpoint (family outcomes separately owed). Newly reachable
  endpoints fail. Other object directories remain untested. Tested system pipes
  include lsarpc, samr, svcctl and epmapper.
* Mapper diagnostics record endpoint-inquiry and MMC20.Application local-server
  activation replies. The target must not already be running before either call;
  a successful control server start must appear in the shared Security 4688
  capture. **Limitation: this uses COM runtime activation, not activation over the
  measured direct mapper connection. The outcome row therefore remains unrun.**
  No client-library prerequisite failure qualifies as service refusal.
  Fold 16 also records the direct ALPC connection's literal status, but sends no
  endpoint-lookup RPC request. The missing ALPC RPC bind, ept lookup NDR request and
  returned-endpoint activation wire implementation is the explicit unrun reason.
  There is no mapper pass branch: API labels and reply flags cannot qualify it.
* GetAddrInfoW, DnsQuery_W and synchronous DnsQueryEx use www.example.com, with a control immediately
  before each confined call in the same run. Literal statuses and Winsock startup
  failures and each call's UTC start timestamp are retained. The inventory verdict
  looks up RPC Control/DNSResolver and reports its LPAC literal status or its
  absence from the enumerated namespace. No resolution can pass only with the completed scoped
  inventory backstop excluding the DNS service from LPAC-reachable endpoints.
* Each ShellExecute variant and a non-HTTP .cmd file target run three pairs. Each
  call has a fresh nonce (in URL or file path). The evaluator requires a control
  nonce-bearing process start in the same Security capture, no confined nonce
  start and no confined listener GET. It permits confined API success. Server
  results remain unrun with "server runner only"; Windows 11 interactive-client
  coverage is NOT MET. The capture helper enables process creation via auditpol
  and command-line auditing via the registry. It requires Success in auditpol's
  readback and publishes the normalized inclusion setting. Filtered-query errors
  fail capture; only the explicit no-matching-events result is an empty success.
  The control's ending record ID is the confined window's starting ID; both query
  upper bounds are fixed before filtering. Each window extends ten seconds after
  its call returns, and the evaluator checks contiguity and the observation period.
  Nonces are matched in Python, never supplied on the capture helper command line.
  Full event XML stays private; only record IDs, bounds and booleans are published.
* Every section/file-view working-set row runs three pairs with a 65,536-byte
  slack. Any excess over cap + slack fails. Every repetition retains its measured
  excess, and the aggregate reports the maximum. Top-level control/confined
  objects explicitly identify their representative repetition. Private commit
  ceilings and best-effort scratch-file budget semantics are unchanged.
* The sweep uses TCP/6 and UDP/17 for IPv4 and IPv6, protocol 0 for other families,
  plus protocol 1 for Hyper-V. It publishes packet type/protocol/flags and literal
  control/plain-AppContainer/LPAC outcomes. These are tested triples, not a full
  installed-provider catalog or proof about untested transports.
