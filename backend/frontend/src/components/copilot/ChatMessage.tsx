import { useState, memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { ThumbsUp, ThumbsDown } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ChatMessage as ChatMessageType } from "@/contexts/CoPilotContext";
import NudgeBanner from "./NudgeBanner";
import ToolResultRenderer from "./ToolResultRenderer";
import ConfirmationCard from "./ConfirmationCard";

interface ChatMessageProps {
  message: ChatMessageType;
  onDismissNudge?: (nudgeId: string, trigger: string, permanent: boolean) => void;
  onConfirmAction?: (confirmId: string) => void;
}

export default memo(function ChatMessage({ message, onDismissNudge, onConfirmAction }: ChatMessageProps) {
  const isUser = message.role === "user";
  const isSystem = message.role === "system";
  const isNudge = message.kind === "nudge" && message.nudge;

  if (isNudge && message.nudge && onDismissNudge) {
    return <NudgeBanner nudge={message.nudge} onDismiss={onDismissNudge} />;
  }

  if (isSystem) {
    return (
      <div className="flex justify-center">
        <span className="text-[11px] text-white/25 px-3 py-1">
          {message.content}
        </span>
      </div>
    );
  }

  if (isUser) {
    return (
      <div className="flex justify-end">
        <div className="max-w-[80%] rounded-2xl rounded-br-sm px-4 py-2.5 text-sm bg-primary/90 text-primary-foreground">
          <span className="whitespace-pre-wrap break-words">{message.content}</span>
        </div>
      </div>
    );
  }

  // Assistant — no bubble, just flowing text + tool results
  return (
    <div className="space-y-1">
      {/* Tool activity indicator */}
      {message.toolStatus && (
        <div className="text-xs text-cyan-400/50 flex items-center gap-1.5">
          <span className="inline-block w-1.5 h-1.5 rounded-full bg-cyan-400/50 animate-pulse" />
          {message.toolStatus}
        </div>
      )}

      {/* Tool results rendered inline */}
      {message.toolResults?.map((tr, i) => (
        <ToolResultRenderer key={i} toolName={tr.toolName} data={tr.data} />
      ))}

      {/* Confirmation card for destructive actions */}
      {message.confirmRequest && !message.confirmResult && onConfirmAction && (
        <ConfirmationCard
          request={message.confirmRequest}
          onConfirm={onConfirmAction}
          onCancel={() => {/* cancel = do nothing, token expires */}}
        />
      )}

      {/* Confirmation result */}
      {message.confirmResult && (
        <div className={cn(
          "my-2 rounded-lg border px-3 py-2 text-xs",
          message.confirmResult.success
            ? "border-green-500/20 bg-green-500/5 text-green-300"
            : "border-red-500/20 bg-red-500/5 text-red-300"
        )}>
          {message.confirmResult.message}
        </div>
      )}

      {/* LLM text content */}
      {message.content && (
        <div className="text-sm text-white/90 leading-relaxed">
          <MarkdownContent content={message.content} isStreaming={message.isStreaming} />
        </div>
      )}

      {/* Streaming cursor when no content yet and no tool activity */}
      {!message.content && message.isStreaming && !message.toolStatus && !message.toolResults?.length && (
        <div className="text-sm text-white/90 leading-relaxed">
          <StreamingCursor />
        </div>
      )}

      {!message.isStreaming && message.usage && (
        <MessageMeta usage={message.usage} />
      )}
    </div>
  );
});

function MarkdownContent({ content, isStreaming }: { content: string; isStreaming?: boolean }) {
  if (!content && isStreaming) {
    return <StreamingCursor />;
  }

  return (
    <div className="copilot-markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          a: ({ children, href, ...props }) => (
            <a href={href} target="_blank" rel="noopener noreferrer" className="text-cyan-400/90 underline underline-offset-2 decoration-cyan-400/30 hover:decoration-cyan-400/60" {...props}>
              {children}
            </a>
          ),
          pre: ({ children, ...props }) => (
            <pre className="bg-white/[0.04] rounded-lg p-3 my-2 overflow-x-auto text-xs border border-white/[0.06]" {...props}>
              {children}
            </pre>
          ),
          code: ({ children, className, ...props }) => {
            const isBlock = className?.startsWith("language-") || className?.startsWith("hljs");
            if (isBlock) return <code className={className} {...props}>{children}</code>;
            return (
              <code className="bg-white/[0.06] rounded px-1.5 py-0.5 text-xs font-mono text-cyan-300/80" {...props}>
                {children}
              </code>
            );
          },
          table: ({ children, ...props }) => (
            <div className="overflow-x-auto my-2">
              <table className="min-w-full text-xs border-collapse" {...props}>{children}</table>
            </div>
          ),
          th: ({ children, ...props }) => (
            <th className="border border-white/10 px-2 py-1 bg-white/[0.04] text-left font-medium text-white/60" {...props}>{children}</th>
          ),
          td: ({ children, ...props }) => (
            <td className="border border-white/10 px-2 py-1 text-white/70" {...props}>{children}</td>
          ),
          ul: ({ children, ...props }) => (
            <ul className="list-disc pl-4 my-1.5 space-y-1 marker:text-white/20" {...props}>{children}</ul>
          ),
          ol: ({ children, ...props }) => (
            <ol className="list-decimal pl-4 my-1.5 space-y-1 marker:text-white/20" {...props}>{children}</ol>
          ),
          p: ({ children, ...props }) => (
            <p className="my-1.5 first:mt-0 last:mb-0 leading-relaxed" {...props}>{children}</p>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
      {isStreaming && <StreamingCursor />}
    </div>
  );
}

function StreamingCursor() {
  return (
    <span
      className="inline-block w-1.5 h-4 bg-cyan-400/50 ml-0.5 align-text-bottom animate-blink rounded-sm"
      aria-label="allAI is typing"
    />
  );
}

function MessageMeta({ usage }: { usage: NonNullable<ChatMessageType["usage"]> }) {
  const [feedback, setFeedback] = useState<"up" | "down" | null>(null);

  return (
    <div className="flex items-center justify-end gap-1 mt-1">
      <button
        onClick={() => setFeedback(feedback === "up" ? null : "up")}
        className={cn("p-0.5 rounded transition-colors", feedback === "up" ? "text-cyan-400/70" : "text-white/15 hover:text-white/30")}
      >
        <ThumbsUp className="h-3 w-3" />
      </button>
      <button
        onClick={() => setFeedback(feedback === "down" ? null : "down")}
        className={cn("p-0.5 rounded transition-colors", feedback === "down" ? "text-red-400/70" : "text-white/15 hover:text-white/30")}
      >
        <ThumbsDown className="h-3 w-3" />
      </button>
    </div>
  );
}
