"""
Re-encrypt LLM API Keys
========================

CLI script for SECRET_KEY rotation.
Decrypts all stored keys with the old secret, re-encrypts with the new one.

Usage:
    python -m app.scripts.reencrypt_llm_keys --old-secret OLD --new-secret NEW

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

import argparse
import sys

from sqlmodel import select

from app.core.database import get_session_context
from app.core.llm_key_crypto import decrypt_api_key, encrypt_api_key
from app.models.llm_settings import LLMSettings


def main() -> None:
    parser = argparse.ArgumentParser(description="Re-encrypt LLM API keys after SECRET_KEY change")
    parser.add_argument("--old-secret", required=True, help="Previous SECRET_KEY")
    parser.add_argument("--new-secret", required=True, help="New SECRET_KEY")
    args = parser.parse_args()

    old_secret: str = args.old_secret
    new_secret: str = args.new_secret

    with get_session_context() as session:
        rows = session.exec(select(LLMSettings)).all()

        if not rows:
            print("No LLM settings found. Nothing to re-encrypt.")
            return

        success = 0
        failed = 0

        for row in rows:
            try:
                # Decrypt with old secret (pass AAD context)
                plaintext = decrypt_api_key(
                    row.encrypted_key, row.key_iv, row.key_tag,
                    old_secret, row.key_version,
                    provider_id=row.provider, scope=row.scope,
                )

                # Re-encrypt with new secret, increment version
                new_version = row.key_version + 1
                ciphertext, iv, tag = encrypt_api_key(
                    plaintext, new_secret, new_version,
                    provider_id=row.provider, scope=row.scope,
                )

                row.encrypted_key = ciphertext
                row.key_iv = iv
                row.key_tag = tag
                row.key_version = new_version
                session.add(row)

                success += 1
                print(f"  OK: {row.provider} (id={row.id}) -> v{new_version}")

            except Exception as e:
                failed += 1
                print(f"  FAIL: {row.provider} (id={row.id}) — {e}", file=sys.stderr)

        if failed == 0:
            session.commit()
            print(f"\nDone. {success} key(s) re-encrypted successfully.")
        else:
            session.rollback()
            print(
                f"\nAborted. {failed} failure(s), {success} would have succeeded. "
                "No changes written (transaction rolled back).",
                file=sys.stderr,
            )
            sys.exit(1)


if __name__ == "__main__":
    main()
