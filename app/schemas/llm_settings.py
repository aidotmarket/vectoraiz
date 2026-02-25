"""
LLM Settings Schemas
====================

Pydantic request/response schemas for LLM admin endpoints.

Phase: BQ-125 — Connect Your LLM
Created: 2026-02-12
"""

from datetime import datetime
from typing import List, Optional

from pydantic import BaseModel, Field


# --- Request schemas ---

class LLMSettingsCreate(BaseModel):
    provider: str = Field(..., min_length=1, max_length=32)
    api_key: str = Field(..., min_length=1, max_length=512)
    model: str = Field(..., min_length=1, max_length=64)
    display_name: Optional[str] = Field(default=None, max_length=128)
    set_active: bool = Field(default=True)


class LLMTestRequest(BaseModel):
    provider: str = Field(..., min_length=1, max_length=32)


# --- Response schemas ---

class LLMSettingsResponse(BaseModel):
    provider: str
    model: str
    display_name: Optional[str] = None
    key_hint: Optional[str] = None
    is_active: bool
    last_tested_at: Optional[datetime] = None
    last_test_ok: Optional[bool] = None
    total_requests: int = 0
    total_tokens: int = 0


class LLMSettingsListResponse(BaseModel):
    configured: bool
    active_provider: Optional[str] = None
    providers: List[LLMSettingsResponse]


class LLMTestResponse(BaseModel):
    ok: bool
    provider: str
    model: Optional[str] = None
    latency_ms: Optional[int] = None
    message: str
    error_code: Optional[str] = None


class LLMModelInfo(BaseModel):
    id: str
    name: str
    context: int
    tier: str


class LLMProviderInfo(BaseModel):
    id: str
    name: str
    models: List[LLMModelInfo]
    key_prefix: str
    docs_url: str


class LLMProvidersResponse(BaseModel):
    providers: List[LLMProviderInfo]


class LLMUsageSummary(BaseModel):
    provider: str
    total_requests: int = 0
    total_tokens: int = 0
    recent_errors: int = 0
