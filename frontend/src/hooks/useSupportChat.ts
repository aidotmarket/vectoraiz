/**
 * hooks/useSupportChat.ts — React hook for allAI Support Chat
 * =============================================================
 *
 * Manages chat state: session lifecycle, message history, SSE streaming,
 * cost tracking, and loading states. Designed for the floating chat widget.
 *
 * FIX (S101): Uses sessionRef to eliminate race condition between
 * session creation and first message send. Persists session_id to
 * localStorage for cross-reload recovery.
 *
 * CREATED: S101 (2026-02-10) — allAI Support Agent Chat UI Integration
 */

import { useState, useCallback, useRef, useEffect } from "react";
import type {
  SupportChatMessage,
  SupportChatSession,
  SessionCostSummary,
} from "../types/supportChat";
import {
  createSession,
  getSession,
  closeSession as apiCloseSession,
  streamMessage,
  sendMessage,
} from "../services/supportChat";

const SESSION_STORAGE_KEY = "allai_support_session_id";

export interface UseSupportChatReturn {
  /** Current session (null if not started) */
  session: SupportChatSession | null;
  /** All messages in current session */
  messages: SupportChatMessage[];
  /** Text currently being streamed (partial assistant response) */
  streamingText: string;
  /** Whether the assistant is currently responding */
  isLoading: boolean;
  /** Cumulative cost summary for the session */
  costSummary: SessionCostSummary | null;
  /** Last error message */
  error: string | null;
  /** Elapsed ms for last response */
  lastElapsedMs: number | null;
  /** Start a new chat session */
  startSession: (topic?: string) => Promise<SupportChatSession | null>;
  /** Resume an existing session by ID */
  resumeSession: (sessionId: string) => Promise<void>;
  /** Send a message — auto-creates session if needed (streaming by default) */
  send: (message: string, stream?: boolean) => Promise<void>;
  /** Close the current session */
  endSession: () => Promise<void>;
  /** Cancel an in-progress stream */
  cancelStream: () => void;
  /** Clear error state */
  clearError: () => void;
}

const EMPTY_COST: SessionCostSummary = {
  total_input_tokens: 0,
  total_output_tokens: 0,
  total_estimated_cost_usd: 0,
  message_count: 0,
};

export function useSupportChat(): UseSupportChatReturn {
  const [session, setSession] = useState<SupportChatSession | null>(null);
  const [messages, setMessages] = useState<SupportChatMessage[]>([]);
  const [streamingText, setStreamingText] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [costSummary, setCostSummary] = useState<SessionCostSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastElapsedMs, setLastElapsedMs] = useState<number | null>(null);

  // Refs to avoid stale closures (fixes race condition)
  const sessionRef = useRef<SupportChatSession | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  // Keep ref in sync with state
  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  // Restore session from localStorage on mount
  useEffect(() => {
    const savedId = localStorage.getItem(SESSION_STORAGE_KEY);
    if (savedId) {
      getSession(savedId)
        .then((sess) => {
          if (sess && sess.status === "active") {
            sessionRef.current = sess;
            setSession(sess);
            setMessages(sess.messages || []);
            setCostSummary(sess.cost_summary || EMPTY_COST);
          } else {
            localStorage.removeItem(SESSION_STORAGE_KEY);
          }
        })
        .catch(() => {
          localStorage.removeItem(SESSION_STORAGE_KEY);
        });
    }
  }, []);

  const startSession = useCallback(async (topic?: string): Promise<SupportChatSession | null> => {
    try {
      setError(null);
      const sess = await createSession({ topic });
      sessionRef.current = sess;
      setSession(sess);
      setMessages(sess.messages || []);
      setCostSummary(sess.cost_summary || EMPTY_COST);
      localStorage.setItem(SESSION_STORAGE_KEY, sess.session_id);
      return sess;
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to create session");
      return null;
    }
  }, []);

  const resumeSession = useCallback(async (sessionId: string) => {
    try {
      setError(null);
      const sess = await getSession(sessionId);
      sessionRef.current = sess;
      setSession(sess);
      setMessages(sess.messages || []);
      setCostSummary(sess.cost_summary || EMPTY_COST);
      localStorage.setItem(SESSION_STORAGE_KEY, sess.session_id);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to resume session");
    }
  }, []);

  const sendWithSession = useCallback(
    async (activeSession: SupportChatSession, message: string, stream: boolean) => {
      // Add user message optimistically
      const userMsg: SupportChatMessage = {
        id: crypto.randomUUID?.() || Date.now().toString(36),
        role: "user",
        content: message,
        timestamp: new Date().toISOString(),
      };
      setMessages((prev) => [...prev, userMsg]);
      setIsLoading(true);
      setError(null);
      setStreamingText("");

      if (stream) {
        // SSE streaming path
        let accumulated = "";
        const controller = streamMessage(
          { session_id: activeSession.session_id, message },
          {
            onDelta: (text) => {
              accumulated += text;
              setStreamingText(accumulated);
            },
            onUsage: (evt) => {
              setCostSummary(evt.cost_summary);
              const assistantMsg: SupportChatMessage = {
                id: crypto.randomUUID?.() || Date.now().toString(36),
                role: "assistant",
                content: accumulated,
                timestamp: new Date().toISOString(),
                token_usage: evt.token_usage,
              };
              setMessages((prev) => [...prev, assistantMsg]);
              setStreamingText("");
            },
            onDone: (evt) => {
              setLastElapsedMs(evt.elapsed_ms);
              setIsLoading(false);
              abortRef.current = null;
            },
            onError: (errMsg) => {
              setError(errMsg);
              setIsLoading(false);
              setStreamingText("");
              abortRef.current = null;
              if (accumulated) {
                const partialMsg: SupportChatMessage = {
                  id: crypto.randomUUID?.() || Date.now().toString(36),
                  role: "assistant",
                  content: accumulated + "\n\n_(response interrupted)_",
                  timestamp: new Date().toISOString(),
                };
                setMessages((prev) => [...prev, partialMsg]);
              }
            },
          },
        );
        abortRef.current = controller;
      } else {
        // Non-streaming path
        try {
          const resp = await sendMessage({
            session_id: activeSession.session_id,
            message,
            stream: false,
          });
          setMessages((prev) => [...prev, resp.message]);
          setCostSummary(resp.cost_summary);
        } catch (err: unknown) {
          setError(err instanceof Error ? err.message : "Failed to send message");
        } finally {
          setIsLoading(false);
        }
      }
    },
    [],
  );

  const send = useCallback(
    async (message: string, stream = true) => {
      // Use ref to get current session (avoids stale closure)
      let active = sessionRef.current;

      // Auto-create session if none exists
      if (!active) {
        const newSess = await startSession("Support");
        if (!newSess) return; // Error already set by startSession
        active = newSess;
      }

      await sendWithSession(active, message, stream);
    },
    [startSession, sendWithSession],
  );

  const cancelStream = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
      setIsLoading(false);
    }
  }, []);

  const endSession = useCallback(async () => {
    const current = sessionRef.current;
    if (!current) return;
    try {
      await apiCloseSession(current.session_id);
      setSession((prev) => (prev ? { ...prev, status: "closed" } : null));
      sessionRef.current = null;
      localStorage.removeItem(SESSION_STORAGE_KEY);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Failed to close session");
    }
  }, []);

  const clearError = useCallback(() => setError(null), []);

  return {
    session,
    messages,
    streamingText,
    isLoading,
    costSummary,
    error,
    lastElapsedMs,
    startSession,
    resumeSession,
    send,
    endSession,
    cancelStream,
    clearError,
  };
}
