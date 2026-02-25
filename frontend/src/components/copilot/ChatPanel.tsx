import { useRef, useEffect, useState, useCallback } from "react";
import { Rnd } from "react-rnd";
import { X } from "lucide-react";
import { useCoPilot } from "@/contexts/CoPilotContext";
import ChatMessage from "./ChatMessage";
import ChatInput from "./ChatInput";

const STORAGE_KEY = "allie-window-state";
const MOBILE_BREAKPOINT = 640;

interface WindowState {
  x: number;
  y: number;
  width: number;
  height: number;
}

function getDefaultState(): WindowState {
  const width = 480;
  const height = 420;
  return {
    x: Math.max(0, (window.innerWidth - width) / 2),
    y: Math.max(0, window.innerHeight - height - 40),
    width,
    height,
  };
}

function loadWindowState(): WindowState {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      const parsed = JSON.parse(saved) as WindowState;
      const maxX = window.innerWidth - 100;
      const maxY = window.innerHeight - 100;
      return {
        x: Math.max(0, Math.min(parsed.x, maxX)),
        y: Math.max(0, Math.min(parsed.y, maxY)),
        width: Math.max(360, Math.min(parsed.width, 900)),
        height: Math.max(280, Math.min(parsed.height, window.innerHeight * 0.85)),
      };
    }
  } catch {
    // ignore
  }
  return getDefaultState();
}

function saveWindowState(state: WindowState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // ignore
  }
}

export default function ChatPanel() {
  const {
    isOpen,
    close,
    messages,
    isStreaming,
    isStandalone,
    allieAvailable,
    connectionStatus,
    reconnectCountdown,
    sendMessage,
    stopStreaming,
    dismissNudge,
    sendConfirmAction,
  } = useCoPilot();

  const scrollRef = useRef<HTMLDivElement>(null);
  const [windowState, setWindowState] = useState<WindowState>(loadWindowState);
  const [isMobile, setIsMobile] = useState(window.innerWidth < MOBILE_BREAKPOINT);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const handleResize = () => setIsMobile(window.innerWidth < MOBILE_BREAKPOINT);
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  useEffect(() => {
    if (isOpen) {
      requestAnimationFrame(() => setVisible(true));
    } else {
      setVisible(false);
    }
  }, [isOpen]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  const handleDragStop = useCallback((_e: any, d: { x: number; y: number }) => {
    setWindowState((prev) => {
      const next = { ...prev, x: d.x, y: d.y };
      saveWindowState(next);
      return next;
    });
  }, []);

  const handleResizeStop = useCallback(
    (_e: any, _dir: any, ref: HTMLElement, _delta: any, position: { x: number; y: number }) => {
      setWindowState(() => {
        const next = {
          x: position.x,
          y: position.y,
          width: parseInt(ref.style.width, 10),
          height: parseInt(ref.style.height, 10),
        };
        saveWindowState(next);
        return next;
      });
    },
    []
  );

  const disabled = isStandalone || !allieAvailable;

  if (!isOpen) return null;

  const panelContent = (
    <>
      {/* Floating close — absolute top-right */}
      <button
        onClick={close}
        className="absolute top-3 right-3 z-10 p-1 rounded-full text-white/40 hover:text-white/80 hover:bg-white/10 transition-all"
        title="Close"
      >
        <X className="h-4 w-4" />
      </button>

      {/* Reconnect notice */}
      {connectionStatus === "disconnected" && reconnectCountdown != null && (
        <div className="px-5 pt-3 pb-1">
          <span className="text-xs text-amber-400/80">
            Reconnecting{reconnectCountdown > 0 ? ` in ${reconnectCountdown}s` : "…"}
          </span>
        </div>
      )}

      {/* Messages */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 pt-4 pb-2 space-y-4 scrollbar-thin">
        {messages.length === 0 && !disabled && (
          <div className="flex items-center justify-center h-full">
            <p className="text-sm text-white/30 italic">Ask me anything about your data…</p>
          </div>
        )}
        {messages.map((msg) => (
          <ChatMessage key={msg.id} message={msg} onDismissNudge={dismissNudge} onConfirmAction={sendConfirmAction} />
        ))}
      </div>

      {/* Input */}
      <ChatInput onSend={sendMessage} onStop={stopStreaming} isStreaming={isStreaming} disabled={disabled} />
    </>
  );

  // Mobile: full-screen overlay
  if (isMobile) {
    return (
      <div
        className={`fixed inset-0 z-50 flex flex-col transition-opacity duration-200 ${
          visible ? "opacity-100" : "opacity-0"
        }`}
        style={{
          background: "rgba(10, 15, 28, 0.64)",
          backdropFilter: "blur(24px)",
          WebkitBackdropFilter: "blur(24px)",
        }}
      >
        {panelContent}
      </div>
    );
  }

  // Desktop: frosted glass floating panel
  return (
    <Rnd
      position={{ x: windowState.x, y: windowState.y }}
      size={{ width: windowState.width, height: windowState.height }}
      minWidth={360}
      minHeight={280}
      maxWidth={900}
      maxHeight={window.innerHeight * 0.85}
      bounds="window"
      dragHandleClassName="allie-drag-zone"
      enableResizing={true}
      onDragStop={handleDragStop}
      onResizeStop={handleResizeStop}
      style={{
        zIndex: 50,
        opacity: visible ? 1 : 0,
        transform: visible ? "translateY(0)" : "translateY(12px)",
        transition: "opacity 200ms ease, transform 200ms ease",
        pointerEvents: visible ? "auto" : "none",
      }}
      className="!fixed"
    >
      <div
        className="relative flex flex-col h-full rounded-2xl overflow-hidden border border-white/[0.08] shadow-[0_8px_40px_rgba(0,0,0,0.5)]"
        style={{
          background: "rgba(12, 17, 30, 0.54)",
          backdropFilter: "blur(28px) saturate(150%)",
          WebkitBackdropFilter: "blur(28px) saturate(150%)",
        }}
      >
        {/* Invisible drag zone — top edge */}
        <div className="allie-drag-zone absolute top-0 left-0 right-10 h-10 cursor-grab active:cursor-grabbing z-[5]" />

        {panelContent}
      </div>
    </Rnd>
  );
}
