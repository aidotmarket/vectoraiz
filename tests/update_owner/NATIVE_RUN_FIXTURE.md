# R374 native Run/Reconcile preparation

SOURCE ONLY at authoring time. No native check has been consumed. Root alone has
an existing exception for one invocation, at most these three cases in order:
`real_run_completion`, `real_run_helper_loss`,
`real_reconcile_after_poller_exit`. Stop on unknown/setup/measurement failure;
do not retry, skip ahead, or count missing Done as success.

`native_run_probe.go` is standalone test glue, **not a production owner
executable**. It must be compiled with byte-exact c721ca2d13d8e96c71304160440acc9b5658f97b
`host-updater/{process.go,storage.go,os_unix.go}` only. No product main, controller,
application, native adapter replacement, or injected policy. Production's true
native guard remains true in this explicit probe; existing TestMain and guarded
tests are unchanged. `command` and `watchdog` dispatch into the actual production
functions with ordinary argv. Run is called once per fresh case; Reconcile is
called once in the final case against the same root and original deadline.

## Root's offline build (not author execution)

Use existing Go **1.27.1 linux/arm64**, Python with Linux pidfd support, and
RTK **0.34.3** spawn/wait topology. Set absolute `R374_SOURCE` to this checkout,
`R374_BUILD` to a newly created private owned build directory,
`R374_OUTPUT` to an existing private owned disposable evidence directory,
and `R374_GO` to the existing Go 1.27.1 executable. These paths are operator
choices, not product metadata. Keep all resulting files private and retained.
All operator process launches, including the build, begin with `rtk proxy`.

Copy the three production files and `native_run_probe.go` into `R374_BUILD`
using `rtk proxy cp` (source checkout remains unchanged). In the shell executing
these commands, export the four variables. Confine RTK bookkeeping for build and
receipt commands too: set `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
`XDG_CACHE_HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR`, and `RTK_DB_PATH` to paths
inside the fresh build directory; create the directory paths with mode 0700.
Do not change PATH to another Go installation.

The complete compile command is:

```sh
rtk proxy env GOOS=linux GOARCH=arm64 CGO_ENABLED=0 GOPROXY=off GOSUMDB=off GO111MODULE=off GOTOOLCHAIN=local GOCACHE="$R374_BUILD/go-cache" "$R374_GO" build -o "$R374_BUILD/probe" "$R374_BUILD/native_run_probe.go" "$R374_BUILD/process.go" "$R374_BUILD/storage.go" "$R374_BUILD/os_unix.go"
```

Prepare the build attestation after that command succeeds. This reads sources
and toolchain metadata; it does not invoke the probe or consume native cases:

```sh
rtk proxy python3 - <<'PY'
import hashlib, json, os, subprocess
from pathlib import Path
build = Path(os.environ['R374_BUILD']).resolve(strict=True)
source = Path(os.environ['R374_SOURCE']).resolve(strict=True)
go = os.environ['R374_GO']
version = subprocess.check_output(['rtk', 'proxy', go, 'version'], text=True).strip()
assert version == 'go version go1.27.1 linux/arm64', version
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
names = ('process.go', 'storage.go', 'os_unix.go', 'native_run_probe.go')
hashes = {n: sha(build / n) for n in names}
for n in names:
    original = source / ('host-updater' if n != 'native_run_probe.go' else 'tests/update_owner') / n
    assert hashes[n] == sha(original), n
hashes['native_run_fixture.py'] = sha(source / 'tests/update_owner/native_run_fixture.py')
hashes['probe'] = sha(build / 'probe')
receipt = dict(base='c721ca2d13d8e96c71304160440acc9b5658f97b',
               toolchain='go1.27.1', go_version=version, go_executable_sha256=sha(Path(go)),
               GOOS='linux', GOARCH='arm64', CGO_ENABLED='0', GOPROXY='off', GOSUMDB='off',
               source_head=subprocess.check_output(['rtk','proxy','git','-C',str(source),'rev-parse','HEAD'], text=True).strip(),
               source_tree=subprocess.check_output(['rtk','proxy','git','-C',str(source),'rev-parse','HEAD^{tree}'], text=True).strip(),
               sha256=hashes,
               command=['rtk','proxy','env','GOOS=linux','GOARCH=arm64','CGO_ENABLED=0',
                        'GOPROXY=off','GOSUMDB=off','GO111MODULE=off','GOTOOLCHAIN=local',
                        'GOCACHE='+str(build/'go-cache'),go,'build','-o',str(build/'probe'),
                        *[str(build/n) for n in ('native_run_probe.go','process.go','storage.go','os_unix.go')]])
p = build / 'build-receipt.json'
p.write_text(json.dumps(receipt, indent=2, sort_keys=True)+'\n')
p.chmod(0o600)
PY
```

## Root's exact three once

No additional permission is needed. Do not run this during source preparation.
One authorized invocation below attempts the three cases sequentially, with no
retries. Do not run help/import/smoke probes first. The probe binary has no
service, install, supervisor, or general arbitrary-command interface.

```sh
rtk proxy python3 "$R374_SOURCE/tests/update_owner/native_run_fixture.py" --native --probe "$R374_BUILD/probe" --build-receipt "$R374_BUILD/build-receipt.json" --output-root "$R374_OUTPUT"
```

Each fresh owned case uses a synthetic environment/input and benign Python file,
private release marker, original 20-second absolute deadline (never renewed),
and command self-cap of 25 seconds. A separate sentinel has a 25-second cap.
The operator independently releases completion. No leaf subprocess is needed.
All operator Popen argv begin `rtk proxy`; actual RTK wrapper and child identities
are pinned separately. Product helper/watchdog/command stay direct argv.

The startup child.lock gate prevents command execution until an external guardian
has inherited pinned pidfds and private control pipes. A second guardian covers
all actual work roles before experimental signaling. Guardians have EOF cleanup
and a 30-second cap, and their own RTK wrappers/actual children are pinned and
reaped. Guardians are protected by pipe EOF/cap rather than circular guardians.
Only pidfd signaling is used by the operator; production watchdog group killing
is the behavior under examination. Discovery reads only verified ancestors'
thread child lists, never a host PID/group scan. UID, parent, argv, process group,
and start identity are checked on both sides of pidfd acquisition.

The helper-loss observation requires the actual Run result and durable Done with
`child_outcome_unknown`, observed within two seconds of the helper-only signal
and before the original deadline. The final case kills only the polling owner,
observes surviving helper work/held child.lock, starts a fresh Reconcile, observes
it pending for at least a sampled 200ms interval with both locks held, then
releases work. The dead owner's result is explicitly unavailable. Every case
requires exactly one execution, released real locks, all work targets/wrappers
ended, and a surviving sentinel before cleanup.

Retain full receipt.json, per-case outcome.json, process.log, work markers and
records, build receipt, binary, and build sources. Cleanup uses pinned pidfds
only, waits retained Popen handles, and preserves files. Unverified early setup
can leave descendant coverage unknown; bounded product/command/guardian lifetimes
are a limit, never cleanup-success evidence. Unknown stops subsequent cases.
Receipt source hashes bind the three production files, wrapper, driver, and
binary; build provenance is an operator attestation. Hash checks also run after
observation. A result-file observation bounds return timing from above; polling
is not a claim about an exact process exit instant.

These checks can prove these Unix Run/Reconcile paths only. Native platform
supervisor, installation, reboot, full one click, Docker, browser, customer
acceptance, and unrelated R289/R328/R339 work remain outside this fixture.
