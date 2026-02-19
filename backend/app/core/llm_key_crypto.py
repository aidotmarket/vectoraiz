"""
LLM API Key Encryption
======================

AES-256-GCM encryption for LLM API keys with HKDF key derivation.
Supports key versioning, AAD binding, and dual-decrypt fallback for SECRET_KEY rotation.

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

from __future__ import annotations

import os
from typing import Optional

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes


def _derive_encryption_key(secret_key: str, key_version: int = 1) -> bytes:
    """Derive a 256-bit encryption key from SECRET_KEY using HKDF."""
    hkdf = HKDF(
        algorithm=hashes.SHA256(),
        length=32,
        salt=f"vectoraiz-llm-keys-v{key_version}".encode(),
        info=b"llm-api-key-encryption",
    )
    return hkdf.derive(secret_key.encode())


def _build_aad(provider_id: Optional[str] = None, scope: Optional[str] = None) -> Optional[bytes]:
    """Build Associated Authenticated Data binding ciphertext to record context.

    AAD prevents DB field swaps — ciphertext encrypted for one provider:scope
    will fail to decrypt if moved to a different row.
    """
    if provider_id is not None and scope is not None:
        return f"{provider_id}:{scope}".encode()
    return None


def encrypt_api_key(
    plaintext_key: str,
    secret_key: str,
    key_version: int = 1,
    provider_id: Optional[str] = None,
    scope: Optional[str] = None,
) -> tuple[bytes, bytes, bytes]:
    """Encrypt an API key. Returns (ciphertext, iv, tag).

    When provider_id and scope are provided, they are bound as AAD
    so the ciphertext can only be decrypted in the same context.
    """
    derived_key = _derive_encryption_key(secret_key, key_version)
    aesgcm = AESGCM(derived_key)
    iv = os.urandom(12)  # 96-bit nonce
    aad = _build_aad(provider_id, scope)
    ct_with_tag = aesgcm.encrypt(iv, plaintext_key.encode(), aad)
    ciphertext = ct_with_tag[:-16]
    tag = ct_with_tag[-16:]
    return ciphertext, iv, tag


def decrypt_api_key(
    ciphertext: bytes,
    iv: bytes,
    tag: bytes,
    secret_key: str,
    key_version: int = 1,
    provider_id: Optional[str] = None,
    scope: Optional[str] = None,
) -> str:
    """Decrypt an API key. Returns plaintext.

    provider_id and scope must match what was used during encryption.
    """
    derived_key = _derive_encryption_key(secret_key, key_version)
    aesgcm = AESGCM(derived_key)
    aad = _build_aad(provider_id, scope)
    plaintext = aesgcm.decrypt(iv, ciphertext + tag, aad)
    return plaintext.decode()


def decrypt_with_fallback(
    ciphertext: bytes,
    iv: bytes,
    tag: bytes,
    current_secret: str,
    previous_secret: Optional[str],
    key_version: int = 1,
    provider_id: Optional[str] = None,
    scope: Optional[str] = None,
) -> str:
    """Try current SECRET_KEY first, fall back to previous."""
    try:
        return decrypt_api_key(ciphertext, iv, tag, current_secret, key_version, provider_id, scope)
    except Exception:
        if previous_secret:
            return decrypt_api_key(ciphertext, iv, tag, previous_secret, key_version, provider_id, scope)
        raise
