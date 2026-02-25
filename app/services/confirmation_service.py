"""
Confirmation Token Service — Server-Enforced Destructive Action Gate

[COUNCIL MANDATE — MP #4: LLM must NEVER control destructive confirmation]

Flow:
1. LLM calls delete_dataset(dataset_id="abc123")
2. Backend does NOT execute. Instead:
   a. Generates confirmation token (UUID, bound to user+action+resource, 60s TTL)
   b. Stores in memory dict (no Redis dependency)
   c. Returns to LLM: {"status": "confirmation_required", "message": "..."}
   d. Sends CONFIRM_REQUEST to frontend via WebSocket
3. Frontend shows confirmation UI (button with dataset name + action)
4. User clicks "Confirm Delete" → sends CONFIRM_ACTION{confirm_id} via WebSocket
5. Backend validates token → executes deletion → sends CONFIRM_RESULT

PHASE: BQ-ALLAI-B0 — Security Infrastructure
CREATED: 2026-02-16
"""

import logging
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable, Awaitable, Dict, Optional

logger = logging.getLogger(__name__)

CONFIRMATION_TTL_SECONDS = 60

# Tools that require human confirmation before execution
DESTRUCTIVE_TOOLS = {"delete_dataset"}


@dataclass
class PendingConfirmation:
    """A pending destructive action awaiting user confirmation."""
    token: str
    user_id: str
    tool_name: str
    tool_input: dict
    session_id: str
    created_at: float
    description: str = ""
    details: Dict[str, Any] = field(default_factory=dict)


class ConfirmationService:
    """
    Server-enforced confirmation for destructive tool calls.

    Uses in-memory dict with TTL cleanup. Thread-safe for single-worker
    uvicorn (current architecture). For multi-worker, migrate to Redis.
    """

    def __init__(self) -> None:
        self._pending: Dict[str, PendingConfirmation] = {}

    def request_confirmation(
        self,
        user_id: str,
        tool_name: str,
        tool_input: dict,
        session_id: str,
        description: str = "",
        details: Optional[Dict[str, Any]] = None,
    ) -> str:
        """
        Create a confirmation token for a destructive action.

        Returns the token UUID string.
        """
        self._cleanup_expired()

        token = str(uuid.uuid4())
        self._pending[token] = PendingConfirmation(
            token=token,
            user_id=user_id,
            tool_name=tool_name,
            tool_input=tool_input,
            session_id=session_id,
            created_at=time.time(),
            description=description,
            details=details or {},
        )

        logger.info(
            "Confirmation requested: token=%s user=%s tool=%s",
            token[:8], user_id, tool_name,
        )
        return token

    def validate_and_execute(
        self,
        token: str,
        user_id: str,
    ) -> Optional[PendingConfirmation]:
        """
        Validate a confirmation token.

        Checks:
        - Token exists and hasn't expired
        - Token belongs to this user
        - Token hasn't been used before (single-use)

        Returns the PendingConfirmation if valid, None if invalid.
        The caller is responsible for executing the actual action.
        """
        self._cleanup_expired()

        pending = self._pending.get(token)
        if not pending:
            logger.warning("Confirmation token not found or expired: %s", token[:8])
            return None

        if pending.user_id != user_id:
            logger.warning(
                "Confirmation token user mismatch: token=%s expected=%s got=%s",
                token[:8], pending.user_id, user_id,
            )
            return None

        # Single-use: remove immediately
        del self._pending[token]

        logger.info(
            "Confirmation validated: token=%s user=%s tool=%s",
            token[:8], user_id, pending.tool_name,
        )
        return pending

    def _cleanup_expired(self) -> None:
        """Remove expired tokens."""
        now = time.time()
        expired = [
            token for token, p in self._pending.items()
            if now - p.created_at > CONFIRMATION_TTL_SECONDS
        ]
        for token in expired:
            del self._pending[token]


# Module-level singleton
confirmation_service = ConfirmationService()
