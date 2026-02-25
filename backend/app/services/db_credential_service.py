"""
Database Credential Service
============================

Fernet encrypt/decrypt for database connection passwords.
Reuses the same SECRET_KEY infrastructure as LLM API keys (BQ-125).

Phase: BQ-VZ-DB-CONNECT
Created: 2026-02-25
"""

import logging
from cryptography.fernet import Fernet

from app.config import settings

logger = logging.getLogger(__name__)


def _get_fernet() -> Fernet:
    """Return a Fernet instance using the app's SECRET_KEY."""
    return Fernet(settings.get_secret_key().encode())


def encrypt_password(plaintext: str) -> str:
    """Encrypt a database password. Returns a URL-safe base64 token string."""
    return _get_fernet().encrypt(plaintext.encode()).decode()


def decrypt_password(token: str) -> str:
    """Decrypt a database password from its Fernet token."""
    return _get_fernet().decrypt(token.encode()).decode()
