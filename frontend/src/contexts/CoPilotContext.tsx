import React, { createContext, useContext, useState, useCallback, useRef, useEffect } from "react";
import { useLocation } from "react-router-dom";
import { useAuth } from "./AuthContext";
import { useMode } from "./ModeContext";
import { getApiUrl, datasetsApi } from "@/lib/api";
import type { ApiDataset } from "@/lib/api";
import { toast } from "sonner";
import type { NudgeData } from "@/components/copilot/NudgeBanner";

export type ToneMode = "professional" | "friendly" | "surfer";

export type ConnectionStatus = "disconnected" | "connecting" | "connected";

/** Generic tool result data (varies by tool_name) */
export type ToolResultData = Record<string, unknown>;

/** Pending confirmation request */
export interface ConfirmRequest {
  confirm_id: string;
  tool_name: string;
  description: string;
  details: Record<string, unknown>;
  expires_in_seconds: number;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: string;
  isStreaming?: boolean;
  kind?: "chat" | "nudge" | "system";
  /** Usage metadata from BRAIN_STREAM_END */
  usage?: { input_tokens?: number; output_tokens?: number; cost_cents?: number; provider?: string; model?: string };
  /** For nudge messages */
  nudge?: NudgeData;
  /** BQ-ALLAI-B: Tool results attached to this message */
  toolResults?: Array<{ toolName: string; data: ToolResultData }>;
  /** BQ-ALLAI-B: Tool currently executing */
  toolStatus?: string;
  /** BQ-ALLAI-B: Pending confirmation request */
  confirmRequest?: ConfirmRequest;
  /** BQ-ALLAI-B: Confirmation result */
  confirmResult?: { confirm_id: string; success: boolean; message: string };
}

interface CoPilotState {
  isOpen: boolean;
  sessionId: string | null;
  messages: ChatMessage[];
  isStreaming: boolean;
  streamingMessageId: string | null;
  streamBuffer: string;
  isConnected: boolean;
  connectionStatus: ConnectionStatus;
  reconnectCountdown: number | null;
  allieAvailable: boolean;
  isStandalone: boolean;
  toneMode: ToneMode;
}

interface CoPilotContextValue extends CoPilotState {
  open: () => void;
  close: () => void;
  toggle: () => void;
  sendMessage: (text: string) => void;
  stopStreaming: () => void;
  setToneMode: (mode: ToneMode) => void;
  dismissNudge: (nudgeId: string, trigger: string, permanent: boolean) => void;
  retryLastMessage: () => void;
  /** BQ-ALLAI-B: Send confirmation action for destructive tool calls */
  sendConfirmAction: (confirmId: string) => void;
}

const CoPilotContext = createContext<CoPilotContextValue | undefined>(undefined);

const TONE_MODE_KEY = "vectoraiz_tone_mode";

export const CoPilotProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { apiKey, isAuthenticated } = useAuth();
  const { isStandalone } = useMode();

  const [isOpen, setIsOpen] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingMessageId, setStreamingMessageId] = useState<string | null>(null);
  const [streamBuffer, setStreamBuffer] = useState("");
  const [isConnected, setIsConnected] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("disconnected");
  const [reconnectCountdown, setReconnectCountdown] = useState<number | null>(null);
  const [allieAvailable, setAllieAvailable] = useState(false);
  const [toneMode, setToneModeState] = useState<ToneMode>(
    () => (localStorage.getItem(TONE_MODE_KEY) as ToneMode) || "friendly"
  );

  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout>>();
  const countdownIntervalRef = useRef<ReturnType<typeof setInterval>>();
  const reconnectAttemptRef = useRef(0);
  const streamBufferRef = useRef("");
  const lastUserMessageRef = useRef<string>("");
  const welcomedThisSessionRef = useRef(false);
  const isOpenRef = useRef(isOpen);
  isOpenRef.current = isOpen;
  const sendStateSnapshotRef = useRef<() => void>(() => {});
  const cachedDatasetsRef = useRef<ApiDataset[]>([]);

  const location = useLocation();

  // Helper: build and send STATE_SNAPSHOT to the backend via WS
  const sendStateSnapshot = useCallback(() => {
    if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return;

    // Extract active_dataset_id from URL patterns like /datasets/:id or ?dataset=:id
    let activeDatasetId: string | null = null;
    const datasetRouteMatch = location.pathname.match(/\/datasets\/([^/]+)/);
    if (datasetRouteMatch) {
      activeDatasetId = datasetRouteMatch[1];
    } else {
      const params = new URLSearchParams(location.search);
      activeDatasetId = params.get("dataset") || null;
    }

    const ws = wsRef.current;

    // Build snapshot with cached datasets first (non-blocking)
    const buildAndSend = (datasets: ApiDataset[]) => {
      if (ws.readyState !== WebSocket.OPEN) return;
      ws.send(
        JSON.stringify({
          type: "STATE_SNAPSHOT",
          current_route: location.pathname,
          page_title: document.title,
          active_dataset_id: activeDatasetId,
          dataset_summary: datasets.map((d) => ({
            id: d.id,
            filename: d.original_filename,
            file_type: d.file_type,
            status: d.status,
            rows: d.metadata?.row_count ?? null,
            columns: d.metadata?.column_count ?? null,
            size_bytes: d.metadata?.size_bytes ?? null,
          })),
          timestamp: new Date().toISOString(),
        })
      );
    };

    // Send immediately with cached data, then refresh in background
    buildAndSend(cachedDatasetsRef.current);

    // Fetch fresh dataset list (non-blocking), re-send if data changed
    datasetsApi.list().then((res) => {
      const fresh = res.datasets || [];
      const stale = cachedDatasetsRef.current;
      cachedDatasetsRef.current = fresh;
      if (fresh.length !== stale.length || fresh.length > 0) {
        buildAndSend(fresh);
      }
    }).catch(() => {
      // Ignore — backend can fallback to DB query
    });
  }, [location.pathname, location.search]);

  // Keep ref in sync so the WS message handler can call the latest version
  sendStateSnapshotRef.current = sendStateSnapshot;

  // Send STATE_SNAPSHOT on route change so allAI knows where the user is
  useEffect(() => {
    sendStateSnapshot();
  }, [sendStateSnapshot]);

  const setToneMode = useCallback((mode: ToneMode) => {
    setToneModeState(mode);
    localStorage.setItem(TONE_MODE_KEY, mode);
  }, []);

  // Restore chat history on mount
  useEffect(() => {
    if (!isAuthenticated || !apiKey) return;

    const restore = async () => {
      try {
        const res = await fetch(`${getApiUrl()}/api/copilot/sessions/current/messages`, {
          headers: { "X-API-Key": apiKey },
        });
        if (res.ok) {
          const data = await res.json();
          if (Array.isArray(data) && data.length > 0) {
            setMessages(
              data.map((m: any) => ({
                id: m.id,
                role: m.role,
                content: m.content,
                createdAt: m.created_at,
                kind: m.kind || "chat",
              }))
            );
          }
        }
      } catch {
        // Ignore restore errors
      }
    };
    restore();
  }, [isAuthenticated, apiKey]);

  const connectWs = useCallback(() => {
    if (!apiKey || !isAuthenticated) return;

    setConnectionStatus("connecting");

    const apiUrl = getApiUrl() || window.location.origin;
    const wsProtocol = apiUrl.startsWith("https") ? "wss" : "ws";
    const wsHost = apiUrl.replace(/^https?:\/\//, "");
    const wsUrl = `${wsProtocol}://${wsHost}/ws/copilot?token=${apiKey}`;

    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      reconnectAttemptRef.current = 0;
      setReconnectCountdown(null);
      if (countdownIntervalRef.current) clearInterval(countdownIntervalRef.current);
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        handleWsMessage(data);
      } catch {
        // Ignore non-JSON messages
      }
    };

    ws.onclose = (event) => {
      setIsConnected(false);
      setConnectionStatus("disconnected");
      wsRef.current = null;

      // Don't reconnect on auth failures or session replacement
      if (event.code === 4001) {
        setMessages((prev) => [
          ...prev,
          {
            id: `err_${Date.now()}`,
            role: "system",
            content: "Authentication failed. Please refresh and sign in again.",
            createdAt: new Date().toISOString(),
          },
        ]);
        return;
      }
      if (event.code === 4002) {
        setMessages((prev) => [
          ...prev,
          {
            id: `err_${Date.now()}`,
            role: "system",
            content: "Session opened in another tab.",
            createdAt: new Date().toISOString(),
          },
        ]);
        return;
      }

      // Reconnect with exponential backoff for normal closures
      const delaySec = Math.min(30, Math.pow(2, reconnectAttemptRef.current));
      reconnectAttemptRef.current += 1;

      // Countdown timer for UI
      let remaining = delaySec;
      setReconnectCountdown(remaining);
      if (countdownIntervalRef.current) clearInterval(countdownIntervalRef.current);
      countdownIntervalRef.current = setInterval(() => {
        remaining -= 1;
        if (remaining <= 0) {
          setReconnectCountdown(null);
          if (countdownIntervalRef.current) clearInterval(countdownIntervalRef.current);
        } else {
          setReconnectCountdown(remaining);
        }
      }, 1000);

      reconnectTimeoutRef.current = setTimeout(connectWs, delaySec * 1000);
    };

    ws.onerror = () => {
      // onclose will fire next — reconnect handled there
    };
  }, [apiKey, isAuthenticated]);

  const handleWsMessage = useCallback((data: any) => {
    switch (data.type) {
      case "CONNECTED":
        setIsConnected(true);
        setConnectionStatus("connected");
        setSessionId(data.session_id);
        setAllieAvailable(data.allie_available ?? false);

        // Send initial STATE_SNAPSHOT so allAI knows where the user is
        sendStateSnapshotRef.current();

        // Auto-open with welcome message on every fresh login
        if (data.allie_available && !welcomedThisSessionRef.current) {
          welcomedThisSessionRef.current = true;
          setMessages((prev) => {
            if (prev.length === 0) {
              return [{
                id: `welcome_${Date.now()}`,
                role: "assistant" as const,
                content: "I'm allAI — your AI data assistant. I can walk you through any of the functions of vectorAIz or help you with data ingestion. Let me know what you need!",
                createdAt: new Date().toISOString(),
              }];
            }
            return prev;
          });
          setIsOpen(true);
        }
        break;

      case "BRAIN_STREAM_CHUNK": {
        const chunk = data.chunk || "";
        streamBufferRef.current += chunk;
        setStreamBuffer(streamBufferRef.current);

        // Update the streaming message in-place
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.id === data.message_id && last.isStreaming) {
            return [
              ...prev.slice(0, -1),
              { ...last, content: streamBufferRef.current },
            ];
          }
          return prev;
        });
        break;
      }

      case "BRAIN_STREAM_END": {
        const fullText = data.full_text || streamBufferRef.current;
        const usage = data.usage || undefined;
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.id === data.message_id && last.isStreaming) {
            return [
              ...prev.slice(0, -1),
              { ...last, content: fullText, isStreaming: false, usage },
            ];
          }
          return prev;
        });
        setIsStreaming(false);
        setStreamingMessageId(null);
        streamBufferRef.current = "";
        setStreamBuffer("");
        break;
      }

      case "STOPPED":
        setIsStreaming(false);
        setStreamingMessageId(null);
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.isStreaming) {
            return [...prev.slice(0, -1), { ...last, isStreaming: false }];
          }
          return prev;
        });
        streamBufferRef.current = "";
        setStreamBuffer("");
        break;

      case "ERROR": {
        setIsStreaming(false);
        setStreamingMessageId(null);
        streamBufferRef.current = "";
        setStreamBuffer("");
        const errorCode = data.code || "";
        let errorContent = data.message || "An error occurred";

        // Map error codes to user-friendly messages
        if (errorCode === "ALLIE_DISABLED") {
          errorContent = "allAI requires an ai.market connection.";
        } else if (errorCode === "RATE_LIMITED") {
          errorContent = "Daily token budget reached. Resets tomorrow.";
        }

        setMessages((prev) => [
          ...prev,
          {
            id: `err_${Date.now()}`,
            role: "system",
            content: errorContent,
            createdAt: new Date().toISOString(),
          },
        ]);
        break;
      }

      case "PING":
        wsRef.current?.send(JSON.stringify({ type: "PONG", nonce: data.nonce }));
        break;

      case "BALANCE_GATE":
        setMessages((prev) => [
          ...prev,
          {
            id: `gate_${Date.now()}`,
            role: "system",
            content: "You've used your ai.market credits. Purchase more to continue chatting with allAI.",
            createdAt: new Date().toISOString(),
          },
        ]);
        break;

      case "NUDGE": {
        // BQ-128 Phase 3: Handle proactive nudge
        const nudgeData: NudgeData = {
          nudge_id: data.nudge_id,
          trigger: data.trigger,
          message: data.message,
          dismissable: data.dismissable ?? true,
          icon: data.icon,
        };

        if (isOpenRef.current) {
          // Panel open: show as inline nudge message in chat
          setMessages((prev) => [
            ...prev,
            {
              id: `nudge_${data.nudge_id}`,
              role: "system",
              content: data.message,
              createdAt: new Date().toISOString(),
              kind: "nudge",
              nudge: nudgeData,
            },
          ]);
        } else {
          // Panel closed: show as toast notification
          toast(data.message, {
            duration: 8000,
            action: {
              label: "Open Chat",
              onClick: () => setIsOpen(true),
            },
          });
          // Also store in messages so it appears when panel opens
          setMessages((prev) => [
            ...prev,
            {
              id: `nudge_${data.nudge_id}`,
              role: "system",
              content: data.message,
              createdAt: new Date().toISOString(),
              kind: "nudge",
              nudge: nudgeData,
            },
          ]);
        }
        break;
      }

      // BQ-ALLAI-B: Tool execution status
      case "TOOL_STATUS": {
        const toolName = data.tool_name || "tool";
        const toolStatusText = data.status === "executing"
          ? `Running ${toolName.replace(/_/g, " ")}...`
          : undefined;

        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.isStreaming) {
            return [
              ...prev.slice(0, -1),
              { ...last, toolStatus: toolStatusText },
            ];
          }
          return prev;
        });
        break;
      }

      // BQ-ALLAI-B: Tool result data for inline rendering
      case "TOOL_RESULT": {
        const toolName = data.tool_name || "unknown";
        const toolData = data.data || {};

        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.isStreaming) {
            const existing = last.toolResults || [];
            return [
              ...prev.slice(0, -1),
              {
                ...last,
                toolResults: [...existing, { toolName, data: toolData }],
                toolStatus: undefined,
              },
            ];
          }
          return prev;
        });
        break;
      }

      // BQ-ALLAI-B: Confirmation request for destructive actions
      case "CONFIRM_REQUEST": {
        const confirmReq: ConfirmRequest = {
          confirm_id: data.confirm_id,
          tool_name: data.tool_name,
          description: data.description,
          details: data.details || {},
          expires_in_seconds: data.expires_in_seconds || 60,
        };

        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.isStreaming) {
            return [
              ...prev.slice(0, -1),
              { ...last, confirmRequest: confirmReq, toolStatus: undefined },
            ];
          }
          return prev;
        });
        break;
      }

      // BQ-ALLAI-B: Confirmation result
      case "CONFIRM_RESULT": {
        const confirmRes = {
          confirm_id: data.confirm_id as string,
          success: data.success as boolean,
          message: data.message as string,
        };

        setMessages((prev) =>
          prev.map((m) =>
            m.confirmRequest?.confirm_id === data.confirm_id
              ? { ...m, confirmResult: confirmRes }
              : m
          )
        );
        break;
      }

      // BQ-ALLAI-B: Heartbeat (no-op, just keeps connection alive)
      case "HEARTBEAT":
        break;

      default:
        break;
    }
  }, []);

  // Connect on mount if authenticated; reset state on logout
  useEffect(() => {
    if (isAuthenticated && apiKey) {
      connectWs();
    } else {
      // Auth dropped — reset all CoPilot state
      setIsOpen(false);
      setMessages([]);
      setIsConnected(false);
      setConnectionStatus("disconnected");
      setAllieAvailable(false);
      setSessionId(null);
      welcomedThisSessionRef.current = false;
    }
    return () => {
      if (reconnectTimeoutRef.current) clearTimeout(reconnectTimeoutRef.current);
      if (countdownIntervalRef.current) clearInterval(countdownIntervalRef.current);
      if (wsRef.current) {
        wsRef.current.onclose = null; // prevent reconnect
        wsRef.current.close();
      }
    };
  }, [isAuthenticated, apiKey, connectWs]);

  const sendMessage = useCallback(
    (text: string) => {
      if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return;
      if (!text.trim()) return;

      lastUserMessageRef.current = text;
      const msgId = `msg_${Date.now().toString(36)}`;

      // Add user message
      setMessages((prev) => [
        ...prev,
        {
          id: `user_${msgId}`,
          role: "user",
          content: text,
          createdAt: new Date().toISOString(),
        },
      ]);

      // Add placeholder streaming assistant message
      setMessages((prev) => [
        ...prev,
        {
          id: msgId,
          role: "assistant",
          content: "",
          createdAt: new Date().toISOString(),
          isStreaming: true,
        },
      ]);

      setIsStreaming(true);
      setStreamingMessageId(msgId);
      streamBufferRef.current = "";
      setStreamBuffer("");

      wsRef.current.send(
        JSON.stringify({
          type: "BRAIN_MESSAGE",
          message: text,
          message_id: msgId,
          client_message_id: `cli_${msgId}`,
        })
      );
    },
    []
  );

  const stopStreaming = useCallback(() => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: "STOP" }));
    }
  }, []);

  const dismissNudge = useCallback((nudgeId: string, trigger: string, permanent: boolean) => {
    // Remove nudge from messages
    setMessages((prev) => prev.filter((m) => m.id !== `nudge_${nudgeId}`));

    // Send dismissal to backend
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(
        JSON.stringify({
          type: "NUDGE_DISMISS",
          nudge_id: nudgeId,
          trigger,
          permanent,
        })
      );
    }
  }, []);

  const retryLastMessage = useCallback(() => {
    if (lastUserMessageRef.current) {
      sendMessage(lastUserMessageRef.current);
    }
  }, [sendMessage]);

  // BQ-ALLAI-B: Send confirmation for destructive actions
  const sendConfirmAction = useCallback((confirmId: string) => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(
        JSON.stringify({
          type: "CONFIRM_ACTION",
          confirm_id: confirmId,
        })
      );
    }
  }, []);

  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  const toggle = useCallback(() => setIsOpen((p) => !p), []);

  return (
    <CoPilotContext.Provider
      value={{
        isOpen,
        sessionId,
        messages,
        isStreaming,
        streamingMessageId,
        streamBuffer,
        isConnected,
        connectionStatus,
        reconnectCountdown,
        allieAvailable,
        isStandalone,
        toneMode,
        open,
        close,
        toggle,
        sendMessage,
        stopStreaming,
        setToneMode,
        dismissNudge,
        retryLastMessage,
        sendConfirmAction,
      }}
    >
      {children}
    </CoPilotContext.Provider>
  );
};

export const useCoPilot = (): CoPilotContextValue => {
  const context = useContext(CoPilotContext);
  if (!context) {
    throw new Error("useCoPilot must be used within a CoPilotProvider");
  }
  return context;
};
