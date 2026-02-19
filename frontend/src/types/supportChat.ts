/**
 * types/supportChat.ts — TypeScript interfaces for allAI Support Chat
 * =====================================================================
 *
 * Maps to backend schemas in app/schemas/support_chat.py
 *
 * CREATED: S101 (2026-02-10) — allAI Support Agent Chat UI Integration
 */

// ---------------------------------------------------------------------------
// Cost Tracking
// ---------------------------------------------------------------------------

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  model: string;
  estimated_cost_usd: number;
}

export interface SessionCostSummary {
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost_usd: number;
  message_count: number;
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

export interface SupportChatMessage {
  id?: string;
  role: "user" | "assistant";
  content: string;
  timestamp: string;
  token_usage?: TokenUsage | null;
  sources_used?: string[] | null;
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export interface SupportChatSession {
  session_id: string;
  user_id: string;
  topic?: string | null;
  status: "active" | "closed";
  messages: SupportChatMessage[];
  cost_summary: SessionCostSummary;
  created_at: string;
  updated_at: string;
  context?: Record<string, unknown> | null;
}

export interface CreateSessionRequest {
  topic?: string;
  context?: Record<string, unknown>;
}

export interface SupportSessionListResponse {
  sessions: SupportChatSession[];
  total: number;
}

// ---------------------------------------------------------------------------
// Message Request / Response
// ---------------------------------------------------------------------------

export interface SupportMessageRequest {
  session_id: string;
  message: string;
  stream?: boolean;
}

export interface SupportMessageResponse {
  session_id: string;
  message: SupportChatMessage;
  cost_summary: SessionCostSummary;
}

// ---------------------------------------------------------------------------
// SSE Event Types
// ---------------------------------------------------------------------------

export interface SSEDeltaEvent {
  text: string;
}

export interface SSEUsageEvent {
  token_usage: TokenUsage;
  cost_summary: SessionCostSummary;
}

export interface SSEDoneEvent {
  elapsed_ms: number;
}

export interface SSEErrorEvent {
  error: string;
}
