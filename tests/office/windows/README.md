Windows C1 feasibility harness (S1940)
====================================

This is a default-off, harness-only measurement seam. It does not qualify C1 or
alter the shipped native worker launcher. Authority: runbooks `origin/main`
`b320acc07315ed76038091a2d71ca50288df7471`, Gate 2 C1 design deltas 2/3.

The `vz-office-windows-harness` crate can be checked independently of native C++:

```
rtk run cargo check --locked -p vz-office-windows-harness --features office-feasibility --target x86_64-pc-windows-msvc
```

Stage A launcher arguments:
`vz-office-windows-launch.exe WORKER confined|control REQUEST.json RESULT.json SCRATCH ATTACK_FILE [EXACT_INPUT ...]`.
All run files belong in a new private evidence root outside the checkout.
The worker is the actual `vz-parser-worker.exe`, built with `office-feasibility`.
The launcher never retries a refused confined launch as an unconfined process.
Positive controls are separately requested invocations.

Each confined invocation creates a fresh zero-capability LPAC profile, grants
its unique SID read/execute to the worker, read to exact input files and write
only to disposable scratch. Windows system runtime ACLs and the profile's own
OS-managed store are intrinsic AppContainer access, not host-folder grants.
Scratch receives a low-integrity label; only this disposable directory is
relabelled. No parent folder, host profile, runtime tree or loopback exemption
is granted. Startup failure is evidence of failure, never of confinement.

The process starts suspended with a three-pipe explicit handle list and a
minimal environment. An intentionally inheritable host-file handle is omitted.
The parent sets/reads back 1.5 GiB process and job committed-memory limits,
kill-on-close, active-process limit one, no breakaway, assigns the job and
checks the effective LPAC token before resuming. Any failure terminates the
worker. There is a deadline, no memory-kill sampler. Job peak commit, working
set and private commit are distinct fields. Temporary DACL grants are restored
and the AppContainer profile deleted after the worker is reaped. The suite
runner owns removal of scratch and host sentinel fixtures, including failures.

Windows has not been executed on the development Mac. Cross-checks establish
Rust/API type correctness only. LPAC startup, actual denials and full format
parity remain unproven until Windows evidence is collected.
