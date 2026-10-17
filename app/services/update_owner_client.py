"""Cooperative, fixed-file owner protocol. Pure stdlib; no Docker/app imports.

The retained Docker socket still confers its existing full authority. This
protocol bounds normal updater behavior; it does not contain a compromised app.
"""
from __future__ import annotations

import contextlib
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import stat
import time

MAILBOX = Path("/run/vectoraiz-owner-mailbox")
_DEFAULT_MAILBOX = MAILBOX
_CANDIDATES = (MAILBOX, Path("/var/lib/vectoraiz-owner-mailbox"), Path("/opt/vectoraiz-owner-mailbox"))
TAG = re.compile(r"v?[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[0-9]+)?\Z")
KEY = re.compile(r"[a-zA-Z0-9_-]{16,128}\Z")


class OwnerError(Exception):
    def __init__(self, code: str, http_status: int = 503):
        self.code, self.http_status = code, http_status
        super().__init__(code)


def _location() -> Path:
    # Owner selects one fixed, image-created, non-mounted path at registration.
    # No arbitrary path arrives from HTTP or bootstrap material.
    if MAILBOX != _DEFAULT_MAILBOX:
        return MAILBOX  # Explicit injection used by the isolated filesystem tests.
    for candidate in _CANDIDATES:
        if (candidate / "bootstrap.json").exists():
            return candidate
    return MAILBOX


def _read(name: str, limit: int = 32768) -> dict:
    # Fixed names supplied by code only. Reject links and bound before decoding.
    fd = os.open(_location() / name, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise OwnerError("invalid_mailbox")
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise OwnerError("mailbox_size")
    return json.loads(raw)


def _write(name: str, data: dict) -> None:
    raw = json.dumps(data, separators=(",", ":"), ensure_ascii=True).encode()
    if len(raw) > 32768:
        raise OwnerError("mailbox_size")
    directory = _location()
    tmp = directory / (".pending-" + secrets.token_hex(12))
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, directory / name)
    finally:
        tmp.unlink(missing_ok=True)


def _sign(pair: dict, kind: str, payload: dict) -> dict:
    raw = json.dumps(payload, separators=(",", ":"), ensure_ascii=True)
    mac = hmac.new(pair["key"].encode(), (kind + "\n" + raw).encode(), hashlib.sha256).hexdigest()
    return {"payload": raw, "mac": mac}


def _verify(pair: dict, kind: str, env: dict) -> dict:
    raw = env["payload"]
    if not isinstance(raw, str) or len(raw.encode()) > 32768:
        raise OwnerError("invalid_envelope")
    expected = hmac.new(pair["key"].encode(), (kind + "\n" + raw).encode(), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(expected, env["mac"]):
        raise OwnerError("invalid_mac")
    return json.loads(raw)


def _pair() -> dict:
    p = _read("bootstrap.json", 4096)
    if set(p) != {"installation", "epoch", "container", "generation", "key"}:
        raise OwnerError("adoption_required")
    if not all(isinstance(p[k], str) and 16 <= len(p[k]) <= 128 for k in ("installation", "epoch", "container", "key")):
        raise OwnerError("invalid_pair")
    if not isinstance(p["generation"], int) or p["generation"] < 0:
        raise OwnerError("invalid_generation")
    return p


def heartbeat() -> bool:
    """A signed random echo proves app-side read AND write on the actual path."""
    try:
        p = _pair()
        _write("ready.json", _sign(p, "ready", p))
        return True
    except (OSError, ValueError, KeyError, TypeError, OwnerError):
        return False


def snapshot(require_fresh: bool = False) -> dict:
    try:
        p = _pair()
        s = _verify(p, "status", _read("status.json", 65536))
        if any(s[k] != p[k] for k in ("installation", "epoch", "container", "generation")):
            raise OwnerError("stale_generation")
        stale = not 0 <= time.time() - s["heartbeat"] <= 15
        if require_fresh and (stale or not s["ready"]):
            raise OwnerError("owner_unavailable")
        s["stale"] = stale
        # Pairing keys never enter this public-to-authenticated-caller projection.
        return s
    except OwnerError:
        raise
    except (OSError, ValueError, KeyError, TypeError):
        raise OwnerError("adoption_required") from None


def available() -> bool:
    try:
        snapshot(require_fresh=True)
        return True
    except OwnerError:
        return False


def status(identifier: str = "current") -> dict:
    s = snapshot()
    candidates = [s.get("current"), *(s.get("history") or [])]
    if identifier == "current":
        return s
    ack = (s.get("keys") or {}).get(identifier)
    op_id = ack["id"] if ack else identifier
    op = next((c for c in candidates if c and c["id"] == op_id), None)
    if not op:
        raise OwnerError("operation_not_found", 404)
    return {**s, "current": op}


@contextlib.contextmanager
def _submission_lock():
    # Container is Linux on every shipped host platform. Covers concurrent API
    # processes too, rather than a thread-only lock over the one-slot mailbox.
    import fcntl
    try:
        fd = os.open(_location() / "submission.lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    except OSError:
        raise OwnerError("adoption_required") from None
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise OwnerError("submission_busy_retry_same_key", 409) from None
        yield
    finally:
        os.close(fd)


def submit(target: str, key: str, principal: str) -> dict:
    if not TAG.fullmatch(target) or not KEY.fullmatch(key) or not principal or len(principal) > 256:
        raise OwnerError("invalid_update_request", 422)
    with _submission_lock():
        s = snapshot()  # A known durable key remains recoverable after expiry.
        known = (s.get("keys") or {}).get(key)
        if known:
            if known["target"] != target:
                raise OwnerError("idempotency_conflict", 409)
            if known.get("code") != 202:
                raise OwnerError("active_operation_conflict", known.get("code", 409))
            return {"status": "accepted", "operation_id": known["id"], "idempotency_key": key}
        snapshot(require_fresh=True)  # Confirmed pre-submission refusal.
        p = _pair()
        message = {"kind": "submit", **{k: p[k] for k in ("installation", "epoch", "container", "generation")},
                   "key": key, "target": target, "principal": principal, "scope": "update:submit",
                   "nonce": secrets.token_hex(24), "expires": int(time.time()) + 90}
        try:
            _write("request.json", _sign(p, "request", message))
        except OSError:
            raise OwnerError("acceptance_unknown", 504) from None
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                env = _verify(p, "ack", _read("ack.json"))
                ack = env["ack"]
                if env["pair"] == p and ack["key"] == key and ack["target"] == target:
                    if ack["code"] != 202:
                        raise OwnerError("active_operation_conflict", 409)
                    return {"status": "accepted", "operation_id": ack["id"], "idempotency_key": key}
            except OwnerError as error:
                if error.http_status == 409:
                    raise
            except (OSError, ValueError, KeyError, TypeError):
                pass  # Incomplete docker cp is neither an ack nor success.
            time.sleep(0.2)
        raise OwnerError("acceptance_unknown", 504)
