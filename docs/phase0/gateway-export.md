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
There is no copy fallback. Folder rename failures stop export with the pack in
staging and no pack files under the source. An explicit SFTP RENAME status error
from the server (including cross-device rejection) likewise leaves the pack in
staging with no pack files under the source. A timeout, transport/IO failure or
unexpected reply instead means **publication outcome unknown**: publication may
or may not have happened. The error names `sftp://user@host:port<destination>`.
List `<source>/vz/<pack-id>`; if it exists with all files, the export completed.
If not, the pack is still in `<staging>/<pack-id>.partial-*` and a retry with the
same pack id is safe. After an acknowledged rename, session-close errors print a
warning and the export still returns success JSON.

Attempts interrupted or failed before publication leave their `.partial-*`
directories in staging.
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
it contains no identity path or key material. Exit codes are 0 for success, 1 for
ordinary failure, and 3 for an unknown SFTP publication outcome. Errors go to
stderr; exit 3 requires the recovery check above before retrying.

Automated checks live in `crates/vz-pack/tests/`. The SFTP test starts a disposable
loopback Paramiko server, authenticates with a generated fixture key, and tests
verified upload, key pinning/rotation refusal, explicit rename rejection/retry,
lost rename acknowledgements (exit-3 error variant with a complete destination),
and encrypted key refusal. Install `paramiko==3.5.1` for local tests;
`VZ_TEST_PYTHON` can select the interpreter. Local runs skip with an explicit message if the server cannot
start; a local `ok` can therefore mean a skip without Paramiko (use `--nocapture`
to see the reason). Native CI installs Paramiko and treats inability to start as
failure on both macOS and Windows. Gateway inventory checks below remain operator work.

## Test gateway runs

Run 2026-10-29 13:15–14:10 CET (S1837) against a production-paired test gateway, following the production self-test in the runbooks (`aim-data-gateway.md`). No listings, purchases or E2E window.

- **Gateway:** `aim-data-gateway` v0.2.4 (image `sha256:11723451…`), gateway `a4d8b866`, paired to the synthetic seller `seller-01`. It ran on GCP VM `aim-gw-selftest-s1837g` (us-central1-a, Debian 12, e2-standard-8). The single source was `/srv/gw/data` (mounted read-only at `/sources/data`). Staging was `/srv/gw/staging`, on the same filesystem and outside the source.
- **Clients:**
  - SFTP export ran from Koskadeux (macOS, release build of this PR at `00a11717`) to `max@VM:22` with an unencrypted ed25519 key.
  - Folder export ran on the VM with a Linux release build of the same commit.
  - Pack: `pack-big` (7 files, 176 MB: `chunks.parquet` 76,202,524 B, `originals-0001.zip` 107,759,253 B, `documents.parquet`, `manifest.json`, `README.md`, `examples/query.py`, `examples/query.json`).
  - A 96 KB pack was used for the SFTP cross-device run.
- **Inventory checks:** the gateway rescans its sources only at connect and then every 15 minutes (`internal/channel/channel.go`, 15-minute ticker). Each check therefore restarted the gateway container, which forces a scan within a few seconds, then listed `GET /api/v1/gateways/{id}/files` as the seller. The API shows `file-<id>` names, so files are matched by size.

| Run | Result |
|---|---|
| Before | Inventory: one 6-byte probe file only |
| SFTP export `pack-g-sftp1` | Host key pinned on first use: `SHA256:mjQOqw/Vk79YHAGhgtuydzYIoa0rC+UcchKv5Fva3w8`, equal to the VM's ED25519 host key. Exit 0, 812 s (~0.22 MB/s end to end; upload ~0.4 MB/s from Koskadeux, plus re-reading every file to hash it). |
| Inventory during that transfer (forced scan at ~23 MB staged) | Probe file only: nothing from the pack |
| Inventory after | Probe + all 7 pack files, sizes equal to the pack |
| SFTP export `pack-g-sftp2` killed (`kill -9`) after 40 s | `.partial-*` (16 MB) left in staging, nothing under `vz/`. Inventory unchanged (same 8 entries) |
| SFTP `pack-g-sftp2` rerun | Exit 0 in 819 s; a second `.partial-*` name was used, the stale one left in staging. Inventory taken while it ran (folder runs below) did not show it; after it: the final inventory had 29 entries: the probe plus four complete packs (7 files each, sizes equal) |
| SFTP cross-device: staging `/dev/shm/vzst` (tmpfs) | Upload finished, then the remote rename failed. Exit 1: "atomic SFTP rename failed (possibly cross-device); nothing was copied into the source". Partial left in `/dev/shm`, nothing under `vz/` |
| Folder export `pack-g-folder1` | Exit 0; destination `/srv/gw/data/vz/pack-g-folder1` |
| Folder export `pack-g-folder2` killed after 0.4 s | 768 KB partial in staging, nothing under `vz/` |
| Folder `pack-g-folder2` rerun | Exit 0 in 1.9 s (176 MB), 7 files |
| Folder cross-device: staging `/dev/shm/vzst` | Refused before copying: "staging and source are on different filesystems; one atomic rename is required". Staging empty, nothing under `vz/` |
| Folder staging inside the source (`/srv/gw/data/.st`) | Refused: "staging must be outside every gateway source" |
| Folder export to an existing pack id | Refused: "export destination already exists or cannot be inspected" |
| Inventory after the folder runs (SFTP rerun still uploading) | 22 entries: the probe plus three complete packs (7 files each, sizes equal). Nothing from the running SFTP export |

**Cleanup:** gateway revoked with `confirm_open_orders`, seller tokens cleared (`api.py logout-all`), `pairing.env` deleted, VM `aim-gw-selftest-s1837g` and firewall rule `aim-gw-selftest-s1837g-web` deleted. Evidence: `/Users/max/koskadeux-state/s1837/g-runs/` on Koskadeux (outputs, inventories).

No inventory was taken during a folder export: the 176 MB export finished in
1.9 s, faster than a forced rescan. The killed folder run stands in for that check.

Folder export to network shares (SMB/NFS) that lack exclusive rename stops with
"filesystem does not support an exclusive (no-replace) rename; nothing was copied
into the source; the pack is in staging". There is no fallback.

**Not covered:** Windows clients (untested this round), SFTP servers other than OpenSSH 9.2 (Debian 12), NFS/SMB shares as the folder target, and a scan landing at the exact moment of the rename.
