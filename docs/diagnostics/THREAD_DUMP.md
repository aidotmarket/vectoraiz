# Thread dumps for investigating hangs

`GET /api/diagnostics/threads` returns Python tracebacks for all threads in the
serving application process. It can help locate blocked code or deadlocks. The
existing `get_current_user` authentication dependency applies.

Invoke the endpoint only under the operator's existing authorization to inspect
that installation. This source repair and its isolated tests do not authorize an
actual diagnostic request. Use the installation's existing authenticated access;
do not place credentials or full dumps in shared logs or reports.

The response remains JSON: `{"thread_dump": "<traceback text>"}`. Capture uses a
private, file-descriptor-backed `tempfile.TemporaryFile`, rewound and decoded as
UTF-8 with replacement for invalid bytes. Its context manager closes and removes
the temporary resource on success or exception. No shared dump file or persistent
debug log is created.

A dump is a snapshot of Python thread stacks, not proof of the cause of a hang.
It does not inspect other processes, generate a diagnostic bundle, establish
bundle delivery, or send anything to ai.market support. External transmission is
a separate explicit action; do not use diagnostic sending to test availability.
If the event loop cannot serve the request, this endpoint may also be unreachable.

For authentication errors, check the existing authenticated session and access
configuration. For a server error, examine the ordinary application error record
for temporary-directory permissions, available disk space, or a faulthandler
write failure. `UnsupportedOperation: fileno` identifies the older in-memory
capture bug; confirm the installed source/version before assuming this repair is
present. A timeout or unavailable response does not establish a particular root
cause. This guide records source behavior, not production acceptance.
