"""
vectorAIz Device Cryptography Module
=====================================

PURPOSE:
    Generate and manage Ed25519 (signing) and X25519 (encryption) keypairs
    for device identity in the Trust Channel architecture.

    Keys are stored in a local JSON keystore, encrypted with a passphrase-derived
    key (PBKDF2HMAC + Fernet). Platform public keys received during registration
    are also stored here.

BQ-102: Refactored from vectoraiz_crypto.py
- Moved to app/core/crypto.py (proper module location)
- Passphrase from env var (no hardcoded secrets)
- Configurable keystore path (persistent Docker volume)
- Platform key storage for post-registration handshake verification
- Atomic file writes to prevent keystore corruption
"""

import os
import json
import base64
import logging
import tempfile
from pathlib import Path
from typing import Optional, Tuple

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ed25519, x25519
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC
from cryptography.hazmat.backends import default_backend
from cryptography.fernet import Fernet

logger = logging.getLogger(__name__)


class DeviceCrypto:
    """
    Manages device cryptographic identity for Trust Channel.

    Provides:
    - Ed25519 keypair generation/loading (for digital signatures)
    - X25519 keypair generation/loading (for encryption/key agreement)
    - Encrypted local keystore (PBKDF2HMAC + Fernet)
    - Platform public key storage (received from ai.market during registration)
    """

    def __init__(self, keystore_path: str, passphrase: str):
        """
        Args:
            keystore_path: Path to the encrypted keystore JSON file.
            passphrase: Passphrase for encrypting/decrypting private keys.
        """
        self.keystore_path = Path(keystore_path)
        self._passphrase = passphrase.encode("utf-8")
        self._pbkdf2_iterations = 600_000  # XAI Council: high iteration count

    # -------------------------------------------------------------------------
    # Key Generation
    # -------------------------------------------------------------------------

    @staticmethod
    def generate_ed25519_keypair() -> Tuple[ed25519.Ed25519PrivateKey, ed25519.Ed25519PublicKey]:
        private_key = ed25519.Ed25519PrivateKey.generate()
        return private_key, private_key.public_key()

    @staticmethod
    def generate_x25519_keypair() -> Tuple[x25519.X25519PrivateKey, x25519.X25519PublicKey]:
        private_key = x25519.X25519PrivateKey.generate()
        return private_key, private_key.public_key()

    # -------------------------------------------------------------------------
    # Private Key Encryption
    # -------------------------------------------------------------------------

    def _derive_fernet_key(self, salt: bytes) -> bytes:
        kdf = PBKDF2HMAC(
            algorithm=hashes.SHA256(),
            length=32,
            salt=salt,
            iterations=self._pbkdf2_iterations,
            backend=default_backend(),
        )
        return base64.urlsafe_b64encode(kdf.derive(self._passphrase))

    def _encrypt_private_key(self, private_key) -> Tuple[bytes, bytes]:
        """Encrypt a private key with passphrase-derived Fernet key."""
        salt = os.urandom(16)
        fernet_key = self._derive_fernet_key(salt)
        f = Fernet(fernet_key)
        raw_bytes = private_key.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )
        return f.encrypt(raw_bytes), salt

    def _decrypt_private_key(self, encrypted_data: bytes, salt: bytes, key_type: str):
        """Decrypt a private key from the keystore."""
        fernet_key = self._derive_fernet_key(salt)
        f = Fernet(fernet_key)
        raw_bytes = f.decrypt(encrypted_data)
        if key_type == "ed25519":
            return ed25519.Ed25519PrivateKey.from_private_bytes(raw_bytes)
        elif key_type == "x25519":
            return x25519.X25519PrivateKey.from_private_bytes(raw_bytes)
        raise ValueError(f"Unknown key type: {key_type}")

    # -------------------------------------------------------------------------
    # Keystore I/O (atomic writes per AG Council review)
    # -------------------------------------------------------------------------

    def _write_keystore(self, data: dict) -> None:
        """Atomically write keystore to prevent corruption."""
        self.keystore_path.parent.mkdir(parents=True, exist_ok=True)
        # Write to temp file in same directory, then atomic rename
        fd, tmp_path = tempfile.mkstemp(
            dir=str(self.keystore_path.parent),
            suffix=".tmp",
        )
        try:
            with os.fdopen(fd, "w") as f:
                json.dump(data, f, indent=2)
            os.replace(tmp_path, str(self.keystore_path))
        except Exception:
            # Clean up temp file on failure
            try:
                os.unlink(tmp_path)
            except OSError:
                pass
            raise

    def _read_keystore(self) -> Optional[dict]:
        """Read keystore, returning None if it doesn't exist."""
        if not self.keystore_path.exists():
            return None
        with open(self.keystore_path, "r") as f:
            return json.load(f)

    # -------------------------------------------------------------------------
    # Public API
    # -------------------------------------------------------------------------

    def get_or_create_keypairs(self) -> Tuple[
        ed25519.Ed25519PrivateKey,
        ed25519.Ed25519PublicKey,
        x25519.X25519PrivateKey,
        x25519.X25519PublicKey,
    ]:
        """
        Load existing keypairs from keystore, or generate new ones.
        Idempotent — safe to call on every startup.

        Returns:
            Tuple of (ed25519_private, ed25519_public, x25519_private, x25519_public)
        """
        keystore = self._read_keystore()

        if keystore and "ed25519_public_key" in keystore:
            try:
                return self._load_keys(keystore)
            except Exception as e:
                logger.error(f"Failed to load keys from keystore: {e}")
                raise

        # Generate fresh keypairs
        logger.info("No existing keypairs found — generating new Ed25519 + X25519 keypairs")
        ed_priv, ed_pub = self.generate_ed25519_keypair()
        x_priv, x_pub = self.generate_x25519_keypair()

        self._save_keys(ed_priv, ed_pub, x_priv, x_pub)
        return ed_priv, ed_pub, x_priv, x_pub

    def _load_keys(self, keystore: dict):
        """Load all four keys from a keystore dict."""
        ed_pub = ed25519.Ed25519PublicKey.from_public_bytes(
            bytes.fromhex(keystore["ed25519_public_key"])
        )
        ed_priv = self._decrypt_private_key(
            keystore["encrypted_ed25519_private_key"].encode("latin-1"),
            bytes.fromhex(keystore["ed25519_salt"]),
            "ed25519",
        )
        x_pub = x25519.X25519PublicKey.from_public_bytes(
            bytes.fromhex(keystore["x25519_public_key"])
        )
        x_priv = self._decrypt_private_key(
            keystore["encrypted_x25519_private_key"].encode("latin-1"),
            bytes.fromhex(keystore["x25519_salt"]),
            "x25519",
        )
        return ed_priv, ed_pub, x_priv, x_pub

    def _save_keys(self, ed_priv, ed_pub, x_priv, x_pub) -> None:
        """Save keypairs to encrypted keystore."""
        enc_ed, ed_salt = self._encrypt_private_key(ed_priv)
        enc_x, x_salt = self._encrypt_private_key(x_priv)

        # Preserve existing platform keys if present
        existing = self._read_keystore() or {}

        data = {
            "ed25519_public_key": ed_pub.public_bytes(
                encoding=serialization.Encoding.Raw,
                format=serialization.PublicFormat.Raw,
            ).hex(),
            "encrypted_ed25519_private_key": enc_ed.decode("latin-1"),
            "ed25519_salt": ed_salt.hex(),
            "x25519_public_key": x_pub.public_bytes(
                encoding=serialization.Encoding.Raw,
                format=serialization.PublicFormat.Raw,
            ).hex(),
            "encrypted_x25519_private_key": enc_x.decode("latin-1"),
            "x25519_salt": x_salt.hex(),
        }

        # Preserve platform keys if already stored
        for key in ("platform_ed25519_public_key", "platform_x25519_public_key", "certificate"):
            if key in existing:
                data[key] = existing[key]

        self._write_keystore(data)
        logger.info(f"Keypairs saved to {self.keystore_path}")

    def get_public_keys_b64(self) -> Tuple[str, str]:
        """Return base64-encoded public keys (for registration request)."""
        keystore = self._read_keystore()
        if not keystore:
            raise RuntimeError("Keystore not initialized — call get_or_create_keypairs() first")
        return (
            base64.b64encode(bytes.fromhex(keystore["ed25519_public_key"])).decode(),
            base64.b64encode(bytes.fromhex(keystore["x25519_public_key"])).decode(),
        )

    def store_platform_keys(
        self,
        platform_ed25519_pub: str,
        platform_x25519_pub: str,
        certificate: str,
    ) -> None:
        """
        Store ai.market platform public keys and certificate after registration.
        Atomic write to prevent partial state (AG Council review).

        Args:
            platform_ed25519_pub: Base64-encoded Ed25519 public key from ai.market
            platform_x25519_pub: Base64-encoded X25519 public key from ai.market
            certificate: Base64-encoded certificate from ai.market
        """
        keystore = self._read_keystore()
        if not keystore:
            raise RuntimeError("Keystore not initialized — cannot store platform keys")

        keystore["platform_ed25519_public_key"] = platform_ed25519_pub
        keystore["platform_x25519_public_key"] = platform_x25519_pub
        keystore["certificate"] = certificate
        self._write_keystore(keystore)
        logger.info("Platform public keys and certificate stored in keystore")

    def has_platform_keys(self) -> bool:
        """Check if platform keys are already stored (for registration skip)."""
        keystore = self._read_keystore()
        if not keystore:
            return False
        return bool(
            keystore.get("platform_ed25519_public_key")
            and keystore.get("platform_x25519_public_key")
            and keystore.get("certificate")
        )
