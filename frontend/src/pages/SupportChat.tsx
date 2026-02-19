/**
 * pages/SupportChat.tsx — allAI Support Chat Widget
 * ====================================================
 *
 * Floating chat widget with:
 * - Expandable/collapsible panel (bottom-right corner)
 * - Session auto-creation on first message
 * - SSE streaming with live text display
 * - Cost awareness display (tokens + USD estimate)
 * - Message history with user/assistant distinction
 * - Loading indicator during streaming
 * - Error display with retry
 *
 * CREATED: S101 (2026-02-10) — allAI Support Agent Chat UI Integration
 */

import { useState, useRef, useEffect, FormEvent } from "react";
import { useSupportChat } from "../hooks/useSupportChat";
import type { SupportChatMessage, SessionCostSummary } from "../types/supportChat";

// ---------------------------------------------------------------------------
// Cost Display Component
// ---------------------------------------------------------------------------

function CostDisplay({ cost }: { cost: SessionCostSummary | null }) {
  if (!cost || cost.message_count === 0) return null;

  const usd = cost.total_estimated_cost_usd;
  const totalTokens = cost.total_input_tokens + cost.total_output_tokens;

  return (
    <div className="flex items-center gap-2 px-3 py-1.5 bg-gray-50 border-t border-gray-200 text-xs text-gray-500">
      <span title="Total tokens used">
        🔢 {totalTokens.toLocaleString()} tokens
      </span>
      <span className="text-gray-300">|</span>
      <span title="Estimated API cost">
        💰 ${usd < 0.01 ? usd.toFixed(4) : usd.toFixed(2)}
      </span>
      <span className="text-gray-300">|</span>
      <span title="Messages exchanged">
        💬 {cost.message_count} {cost.message_count === 1 ? "msg" : "msgs"}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Single Message Bubble
// ---------------------------------------------------------------------------

function MessageBubble({ msg }: { msg: SupportChatMessage }) {
  const isUser = msg.role === "user";

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"} mb-3`}>
      <div
        className={`max-w-[80%] rounded-lg px-3.5 py-2.5 text-sm leading-relaxed ${
          isUser
            ? "bg-indigo-600 text-white rounded-br-sm"
            : "bg-white text-gray-800 border border-gray-200 rounded-bl-sm shadow-sm"
        }`}
      >
        {/* Render content with basic line breaks */}
        {msg.content.split("\n").map((line, i) => (
          <span key={i}>
            {line}
            {i < msg.content.split("\n").length - 1 && <br />}
          </span>
        ))}

        {/* Per-message cost (assistant only) */}
        {!isUser && msg.token_usage && (
          <div className="mt-1.5 pt-1.5 border-t border-gray-100 text-[10px] text-gray-400">
            {msg.token_usage.input_tokens + msg.token_usage.output_tokens} tokens
            {" · "}${msg.token_usage.estimated_cost_usd < 0.01
              ? msg.token_usage.estimated_cost_usd.toFixed(4)
              : msg.token_usage.estimated_cost_usd.toFixed(3)}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Streaming Indicator
// ---------------------------------------------------------------------------

function StreamingBubble({ text }: { text: string }) {
  return (
    <div className="flex justify-start mb-3">
      <div className="max-w-[80%] rounded-lg rounded-bl-sm px-3.5 py-2.5 text-sm leading-relaxed bg-white text-gray-800 border border-gray-200 shadow-sm">
        {text ? (
          text.split("\n").map((line, i) => (
            <span key={i}>
              {line}
              {i < text.split("\n").length - 1 && <br />}
            </span>
          ))
        ) : (
          <span className="flex items-center gap-1 text-gray-400">
            <span className="animate-pulse">●</span>
            <span className="animate-pulse" style={{ animationDelay: "0.2s" }}>●</span>
            <span className="animate-pulse" style={{ animationDelay: "0.4s" }}>●</span>
          </span>
        )}
        <span className="inline-block w-0.5 h-4 bg-indigo-500 animate-pulse ml-0.5 align-text-bottom" />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Chat Widget
// ---------------------------------------------------------------------------

export default function SupportChat() {
  const [isOpen, setIsOpen] = useState(false);
  const [inputValue, setInputValue] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const {
    session,
    messages,
    streamingText,
    isLoading,
    costSummary,
    error,
    startSession,
    send,
    endSession,
    cancelStream,
    clearError,
  } = useSupportChat();

  // Auto-scroll to bottom on new messages or streaming text
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, streamingText]);

  // Focus input when panel opens
  useEffect(() => {
    if (isOpen) {
      setTimeout(() => inputRef.current?.focus(), 100);
    }
  }, [isOpen]);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    const text = inputValue.trim();
    if (!text || isLoading) return;

    setInputValue("");

    // Auto-create session on first message
    if (!session) {
      await startSession("Support");
      // Wait a tick for session state to settle, then send
      // The send will use the session from the next render
    }

    // If session exists (or was just created), send
    if (session) {
      await send(text);
    } else {
      // Session was just created — need to use a small delay
      // This handles the race between setState and send
      setTimeout(async () => {
        await send(text);
      }, 50);
    }
  };

  // Wrapper that ensures session exists before sending
  return (
    <>
      {/* Floating toggle button */}
      {!isOpen && (
        <button
          onClick={() => setIsOpen(true)}
          className="fixed bottom-6 right-6 z-50 w-14 h-14 rounded-full bg-indigo-600 text-white shadow-lg hover:bg-indigo-700 hover:shadow-xl transition-all flex items-center justify-center"
          title="Chat with allAI Support"
        >
          <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z" />
          </svg>
        </button>
      )}

      {/* Chat panel */}
      {isOpen && (
        <div className="fixed bottom-6 right-6 z-50 w-96 h-[32rem] bg-white rounded-xl shadow-2xl border border-gray-200 flex flex-col overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 bg-indigo-600 text-white">
            <div className="flex items-center gap-2">
              <div className="w-2 h-2 rounded-full bg-green-400 animate-pulse" />
              <span className="font-semibold text-sm">allAI Support</span>
            </div>
            <div className="flex items-center gap-1">
              {session && session.status === "active" && (
                <button
                  onClick={endSession}
                  className="p-1.5 rounded hover:bg-indigo-500 transition-colors"
                  title="End session"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 10a1 1 0 011-1h4a1 1 0 011 1v4a1 1 0 01-1 1h-4a1 1 0 01-1-1v-4z" />
                  </svg>
                </button>
              )}
              <button
                onClick={() => setIsOpen(false)}
                className="p-1.5 rounded hover:bg-indigo-500 transition-colors"
                title="Minimize chat"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
              </button>
            </div>
          </div>

          {/* Messages area */}
          <div className="flex-1 overflow-y-auto p-4 bg-gray-50">
            {/* Welcome message if no messages yet */}
            {messages.length === 0 && !isLoading && (
              <div className="text-center py-8">
                <div className="text-3xl mb-2">🤖</div>
                <h3 className="font-semibold text-gray-700 mb-1">
                  Hi! I'm allAI
                </h3>
                <p className="text-sm text-gray-500 max-w-[250px] mx-auto">
                  I can help with marketplace questions, listings, orders, billing, and more.
                </p>
              </div>
            )}

            {/* Message bubbles */}
            {messages.map((msg, i) => (
              <MessageBubble key={msg.id || i} msg={msg} />
            ))}

            {/* Streaming response */}
            {isLoading && <StreamingBubble text={streamingText} />}

            {/* Error display */}
            {error && (
              <div className="mb-3 p-2.5 bg-red-50 border border-red-200 rounded-lg text-xs text-red-700">
                <div className="flex items-center justify-between">
                  <span>⚠️ {error}</span>
                  <button
                    onClick={clearError}
                    className="text-red-500 hover:text-red-700 ml-2"
                  >
                    ✕
                  </button>
                </div>
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>

          {/* Cost display */}
          <CostDisplay cost={costSummary} />

          {/* Input area */}
          <form
            onSubmit={handleSubmit}
            className="flex items-center gap-2 p-3 border-t border-gray-200 bg-white"
          >
            <input
              ref={inputRef}
              type="text"
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              placeholder={
                session?.status === "closed"
                  ? "Session ended"
                  : "Ask allAI anything..."
              }
              disabled={isLoading || session?.status === "closed"}
              className="flex-1 px-3 py-2 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent disabled:bg-gray-100 disabled:cursor-not-allowed"
              maxLength={4000}
            />
            {isLoading ? (
              <button
                type="button"
                onClick={cancelStream}
                className="px-3 py-2 text-sm font-medium text-red-600 bg-red-50 rounded-lg hover:bg-red-100 transition-colors"
                title="Stop generating"
              >
                ■
              </button>
            ) : (
              <button
                type="submit"
                disabled={!inputValue.trim() || session?.status === "closed"}
                className="px-3 py-2 text-sm font-medium text-white bg-indigo-600 rounded-lg hover:bg-indigo-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                Send
              </button>
            )}
          </form>
        </div>
      )}
    </>
  );
}
