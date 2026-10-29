# Gateway export (Phase 0 chunk G)

`vectoraiz-cli export` verifies an existing pack without loading models. The
reusable implementation is `vz_pack::export`. Pack contents and format are
unchanged. Export includes every payload file, nested `examples/` files, and
`manifest.json`; unmanifested files, links and special files are refused.

Configure an existing source directory and an existing staging directory on the
same filesystem. **Staging must be outside every configured gateway source**:
the gateway inventories regular files even in hidden directories. The CLI checks
the supplied source, including canonical paths; the operator must check other
gateway sources too. The gateway does not need to read staging.

```sh
vectoraiz-cli export ./pack --pack-id pack-example \
  --folder /srv/gateway-source --staging /srv/gateway-staging

vectoraiz-cli export ./pack --pack-id pack-example \
  --sftp seller@gateway.example:22:/srv/gateway-source \
  --sftp-staging /srv/gateway-staging --identity /path/to/unencrypted-key \
  --known-hosts /path/to/gateway-known-hosts.json
```

Pack IDs contain 1–128 ASCII letters, digits, hyphens or underscores. They select
the destination directory and do not rewrite the pack's manifest ID. The target
is `<source>/vz/<pack-id>/`. Existing destinations are refused.

Each attempt creates a fresh `<staging>/<pack-id>.partial-<random>/` directory.
Folder export flushes files to disk and checks every copied file's size and
SHA-256. It compares Unix device IDs or Windows volume serials before copying.
SFTP checks remote sizes with stat and re-reads each uploaded file to hash it.
Only after all checks pass does one directory rename expose the complete pack.
There is no copy fallback. A cross-device or other rename failure stops export
and leaves the pack in staging, with no pack files under the source.

Interrupted and failed attempts leave their `.partial-*` directories in staging.
A retry uses a new random name and succeeds without removing old partials. Once
no export is running, operators can remove stale partials. An empty `vz/`
directory may have been created in the source before a failure.

SFTP uses the pinned Apache-2.0 crates `russh = 0.65.0` (ring backend) and
`russh-sftp = 2.1.1`, with no system libssh2/OpenSSL requirement added by SFTP.
The SFTP 2.x pin retains compatibility with LanceDB's pinned Chrono version.
Only unencrypted private keys are supported; password, passphrase and SSH-agent
authentication are unavailable. Without `--identity`, the CLI uses
`~/.ssh/id_ed25519`. It never prints keys or passphrases.

On first connection the CLI writes the host's SHA-256 fingerprint to a JSON pin
file and prints the fingerprint to stderr. Verify it through a trusted channel
on first use. Subsequent connections refuse a different key. `--known-hosts`
selects this app-specific JSON file, **not** OpenSSH's known_hosts format. The
default is `gateway-known-hosts.json` under:

- macOS: `~/Library/Application Support/com.vectoraiz.native/`
- Windows: `%APPDATA%/com.vectoraiz.native/`
- Other development hosts: `$XDG_DATA_HOME/com.vectoraiz.native/` or
  `~/.local/share/com.vectoraiz.native/`.

Do not delete a pin to bypass a mismatch without verifying the server's new key.
Pin updates use an OS file lock (automatically released on process death) on a
persistent `.lock` file and write a complete randomly named `.partial-*` file
before replacing the pin file. Interrupted pin writes are ignored on retry;
stale partials can be removed when no export is running. Keep the lock file.
Remote paths must be absolute POSIX paths; dot components
are normalized and remote canonical paths are checked for staging aliases.
The server must support SFTP v3 directory rename on the same filesystem.

Success prints one JSON object to stdout with `pack_id`, `mode`, `files`
(`path`, `size`, `sha256`, including the manifest), `destination`, and
`elapsed_ms`. The SFTP destination contains the user, host, port and remote path;
it contains no identity path or key material. Errors go to stderr and exit nonzero.

Automated checks live in `crates/vz-pack/tests/`. The SFTP test starts a disposable
loopback Paramiko server, authenticates with a generated fixture key, and tests
verified upload, key pinning/rotation refusal, rename failure/retry and encrypted
key refusal. Install `paramiko==3.5.1` for local tests; `VZ_TEST_PYTHON` can select
the interpreter. Local runs skip with an explicit message if the server cannot
start; Native CI installs Paramiko and treats inability to start as failure on
both macOS and Windows. Gateway inventory checks below remain operator work.

## Test gateway runs
