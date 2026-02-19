/**
 * services/supportChat.ts — allAI Support Chat API client
 * =========================================================
 *
 * Handles HTTP + SSE communication with /api/allai/support endpoints.
 * Provides session CRUD, non-streaming message send, and streaming
 * SSE message send with event parsing.
 *
 * ARCHITECTURE:
 *   - All endpoints require JWT auth (Authorization: Bearer <token>)
 *   - Streaming uses fetch + ReadableStream (no EventSource — POST not supported)
 *   - SSE events: delta, usage, done, error
 *   - Session persistence is server-side; client stores session_id
 *
 * CREATED: S101 (2026-02-10) — allAI Support Agent Chat UI Integration
 */

import type {
  CreateSessionRequest,
  SupportChatSession,
  SupportSessionListResponse,
  SupportMessageRequest,
  SupportMessageResponse,
  SSEDeltaEvent,
  SSEUsageEvent,
  SSEDoneEvent,
  SSEErrorEvent,
} from "../types/supportChat";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const API_BASE =
  import.meta.env.VITE_API_BASE_URL ||
  (import.meta.env.DEV ? "http://localhost:8000" : "");

const SUPPORT_API = `${API_BASE}/api/allai/support`;

function getAuthToken(): string | null {
  return localStorage.getItem("auth_token") || localStorage.getItem("token");
}

function authHeaders(): Record<string, string> {
  const token = getAuthToken();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

export class SupportChatApiError extends Error {
  status: number;
  detail: string;

  constructor(status: number, detail: string) {
    super(`Support API error (${status}): ${detail}`);
    this.name = "SupportChatApiError";
    this.status = status;
    this.detail = detail;
  }
}

async function handleResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let detail = response.statusText;
    try {
      const body = await response.json();
      detail = body.detail || body.message || JSON.stringify(body);
    } catch {
      // body is not JSON
    }
    throw new SupportChatApiError(response.status, detail);
  }
  return response.json();
}

// ---------------------------------------------------------------------------
// Session CRUD
// ---------------------------------------------------------------------------

/** Create a new allAI support chat session. */
export async function createSession(
  request: CreateSessionRequest = {},
): Promise<SupportChatSession> {
  const resp = await fetch(`${SUPPORT_API}/session`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify(request),
  });
  return handleResponse<SupportChatSession>(resp);
}

/** Retrieve a session with full message history. */
export async function getSession(
  sessionId: string,
): Promise<SupportChatSession> {
  const resp = await fetch(`${SUPPORT_API}/session/${sessionId}`, {
    headers: authHeaders(),
  });
  return handleResponse<SupportChatSession>(resp);
}

/** List the authenticated user's sessions. */
export async function listSessions(
  limit = 20,
  offset = 0,
): Promise<SupportSessionListResponse> {
  const params = new URLSearchParams({
    limit: String(limit),
    offset: String(offset),
  });
  const resp = await fetch(`${SUPPORT_API}/sessions?${params}`, {
    headers: authHeaders(),
  });
  return handleResponse<SupportSessionListResponse>(resp);
}

/** Close (archive) a session. */
export async function closeSession(
  sessionId: string,
): Promise<{ status: string; session_id: string }> {
  const resp = await fetch(`${SUPPORT_API}/session/${sessionId}`, {
    method: "DELETE",
    headers: authHeaders(),
  });
  return handleResponse(resp);
}

// ---------------------------------------------------------------------------
// Non-streaming message
// ---------------------------------------------------------------------------

/** Send a message and receive the full response (no streaming). */
export async function sendMessage(
  request: SupportMessageRequest,
): Promise<SupportMessageResponse> {
  const resp = await fetch(`${SUPPORT_API}/message`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify({ ...request, stream: false }),
  });
  return handleResponse<SupportMessageResponse>(resp);
}

// ---------------------------------------------------------------------------
// SSE Streaming message
// ---------------------------------------------------------------------------

/**
 * SSE event callbacks for streaming responses.
 */
export interface StreamCallbacks {
  /** Called for each text chunk. */
  onDelta: (text: string) => void;
  /** Called once after completion with token/cost data. */
  onUsage: (event: SSEUsageEvent) => void;
  /** Called when stream is complete. */
  onDone: (event: SSEDoneEvent) => void;
  /** Called on error. */
  onError: (error: string) => void;
}

/**
 * Send a message and stream the response via SSE.
 *
 * Uses fetch + ReadableStream because EventSource only supports GET.
 * Returns an AbortController so the caller can cancel the stream.
 */
export function streamMessage(
  request: Omit<SupportMessageRequest, "stream">,
  callbacks: StreamCallbacks,
): AbortController {
  const controller = new AbortController();

  const doStream = async () => {
    try {
      const resp = await fetch(`${SUPPORT_API}/message`, {
        method: "POST",
        headers: authHeaders(),
        body: JSON.stringify({ ...request, stream: true }),
        signal: controller.signal,
      });

      if (!resp.ok) {
        let detail = resp.statusText;
        try {
          const body = await resp.json();
          detail = body.detail || detail;
        } catch {
          // not JSON
        }
        callbacks.onError(`API error (${resp.status}): ${detail}`);
        return;
      }

      if (!resp.body) {
        callbacks.onError("Response body is null — streaming not supported");
        return;
      }

      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });

        // Parse SSE events from buffer
        // SSE format: "event: <type>\ndata: <json>\n\n"
        const events = buffer.split("\n\n");
        // Keep the last (potentially incomplete) chunk in buffer
        buffer = events.pop() || "";

        for (const event of events) {
          if (!event.trim()) continue;

          let eventType = "message";
          let eventData = "";

          for (const line of event.split("\n")) {
            if (line.startsWith("event: ")) {
              eventType = line.slice(7).trim();
            } else if (line.startsWith("data: ")) {
              eventData = line.slice(6);
            }
          }

          if (!eventData) continue;

          try {
            const parsed = JSON.parse(eventData);

            switch (eventType) {
              case "delta": {
                const delta = parsed as SSEDeltaEvent;
                callbacks.onDelta(delta.text);
                break;
              }
              case "usage": {
                const usage = parsed as SSEUsageEvent;
                callbacks.onUsage(usage);
                break;
              }
              case "done": {
                const doneEvt = parsed as SSEDoneEvent;
                callbacks.onDone(doneEvt);
                break;
              }
              case "error": {
                const errEvt = parsed as SSEErrorEvent;
                callbacks.onError(errEvt.error);
                break;
              }
              default:
                // Unknown event type — ignore
                break;
            }
          } catch {
            // JSON parse error — skip this event
          }
        }
      }
    } catch (err: unknown) {
      if (err instanceof DOMException && err.name === "AbortError") {
        // Stream was cancelled by user — not an error
        return;
      }
      const message =
        err instanceof Error ? err.message : "Unknown streaming error";
      callbacks.onError(message);
    }
  };

  doStream();
  return controller;
}
