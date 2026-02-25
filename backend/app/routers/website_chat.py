"""
Website Chat Router
===================

Public endpoint for the vectorAIz website chat widget.
No API key required — rate-limited by IP instead.

Created: 2026-02-19
"""

import logging
import time
import uuid
from collections import defaultdict
from typing import Optional

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from app.prompts.website_chat import WEBSITE_CHAT_SYSTEM_PROMPT
from app.services.llm_service import LLMService

logger = logging.getLogger(__name__)

router = APIRouter()

# ---------------------------------------------------------------------------
# Rate limiting (in-memory, per-IP)
# ---------------------------------------------------------------------------
RATE_LIMIT_PER_HOUR = 20
RATE_LIMIT_PER_MINUTE = 3

# { ip: [timestamp, ...] }
_rate_log: dict[str, list[float]] = defaultdict(list)


def _check_rate_limit(ip: str) -> None:
    """Raise 429 if IP exceeds rate limits."""
    now = time.time()
    timestamps = _rate_log[ip]

    # Prune entries older than 1 hour
    _rate_log[ip] = [ts for ts in timestamps if now - ts < 3600]
    timestamps = _rate_log[ip]

    # Check per-minute (last 60s)
    recent_minute = [ts for ts in timestamps if now - ts < 60]
    if len(recent_minute) >= RATE_LIMIT_PER_MINUTE:
        raise HTTPException(
            status_code=429,
            detail="Rate limit exceeded — please wait a moment before sending another message.",
        )

    # Check per-hour
    if len(timestamps) >= RATE_LIMIT_PER_HOUR:
        raise HTTPException(
            status_code=429,
            detail="Rate limit exceeded — you've reached the hourly message limit. Please try again later.",
        )

    # Record this request
    _rate_log[ip].append(now)


# ---------------------------------------------------------------------------
# Conversation memory (in-memory with 30-min expiry)
# ---------------------------------------------------------------------------
CONVERSATION_TTL_S = 30 * 60  # 30 minutes

# { conversation_id: { "messages": [...], "last_active": float } }
_conversations: dict[str, dict] = {}


def _get_conversation(conversation_id: Optional[str]) -> tuple[str, list[dict]]:
    """Get or create a conversation. Returns (conversation_id, messages)."""
    now = time.time()

    # Lazy cleanup: prune expired conversations
    expired = [
        cid for cid, conv in _conversations.items()
        if now - conv["last_active"] > CONVERSATION_TTL_S
    ]
    for cid in expired:
        del _conversations[cid]

    if conversation_id and conversation_id in _conversations:
        conv = _conversations[conversation_id]
        conv["last_active"] = now
        return conversation_id, conv["messages"]

    # New conversation
    new_id = str(uuid.uuid4())
    _conversations[new_id] = {"messages": [], "last_active": now}
    return new_id, _conversations[new_id]["messages"]


# ---------------------------------------------------------------------------
# Request / Response models
# ---------------------------------------------------------------------------
class WebsiteChatRequest(BaseModel):
    message: str = Field(..., min_length=1, max_length=2000)
    conversation_id: Optional[str] = None


class WebsiteChatResponse(BaseModel):
    reply: str
    conversation_id: str


# ---------------------------------------------------------------------------
# Endpoint
# ---------------------------------------------------------------------------
MAX_HISTORY_TURNS = 10  # Keep last N exchanges to avoid token bloat


@router.post(
    "",
    response_model=WebsiteChatResponse,
    summary="Website chat with allAI",
    description="Public chat endpoint for the vectorAIz website. No API key required.",
)
async def website_chat(request: WebsiteChatRequest, http_request: Request):
    # Rate limit by client IP
    client_ip = http_request.client.host if http_request.client else "unknown"
    _check_rate_limit(client_ip)

    # Get or create conversation
    conversation_id, messages = _get_conversation(request.conversation_id)

    # Append user message
    messages.append({"role": "user", "content": request.message})

    # Trim to last N turns to keep context window manageable
    trimmed = messages[-(MAX_HISTORY_TURNS * 2):]

    # Build prompt from conversation history
    prompt_parts = []
    for msg in trimmed:
        role_label = "User" if msg["role"] == "user" else "allAI"
        prompt_parts.append(f"{role_label}: {msg['content']}")
    prompt_parts.append("allAI:")

    prompt = "\n\n".join(prompt_parts)

    try:
        llm = LLMService(provider_override="anthropic")
        reply = await llm.generate(
            prompt=prompt,
            system_prompt=WEBSITE_CHAT_SYSTEM_PROMPT,
            temperature=0.5,
            max_tokens=512,
        )
        reply = reply.strip()
    except Exception as e:
        logger.error("Website chat LLM error: %s", e, exc_info=True)
        raise HTTPException(
            status_code=503,
            detail="allAI is temporarily unavailable. Please try again in a moment.",
        )

    # Store assistant reply
    messages.append({"role": "assistant", "content": reply})

    return WebsiteChatResponse(reply=reply, conversation_id=conversation_id)
