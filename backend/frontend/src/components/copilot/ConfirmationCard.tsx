/**
 * ConfirmationCard — Renders CONFIRM_REQUEST as an inline card.
 *
 * Shows:
 * - Description of the destructive action
 * - Confirm / Cancel buttons
 * - Countdown timer showing TTL
 *
 * PHASE: BQ-ALLAI-B6 — Frontend Tool Result Rendering
 * CREATED: 2026-02-16
 */

import { useState, useEffect, memo } from "react";
import { AlertTriangle } from "lucide-react";

export interface ConfirmRequestData {
  confirm_id: string;
  tool_name: string;
  description: string;
  details: Record<string, unknown>;
  expires_in_seconds: number;
}

interface ConfirmationCardProps {
  request: ConfirmRequestData;
  onConfirm: (confirmId: string) => void;
  onCancel: (confirmId: string) => void;
}

export default memo(function ConfirmationCard({ request, onConfirm, onCancel }: ConfirmationCardProps) {
  const [remaining, setRemaining] = useState(request.expires_in_seconds);
  const [status, setStatus] = useState<"pending" | "confirmed" | "cancelled" | "expired">("pending");

  useEffect(() => {
    if (status !== "pending") return;

    const interval = setInterval(() => {
      setRemaining((prev) => {
        if (prev <= 1) {
          setStatus("expired");
          clearInterval(interval);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => clearInterval(interval);
  }, [status]);

  const handleConfirm = () => {
    setStatus("confirmed");
    onConfirm(request.confirm_id);
  };

  const handleCancel = () => {
    setStatus("cancelled");
    onCancel(request.confirm_id);
  };

  return (
    <div className="my-2 rounded-lg border border-yellow-500/30 bg-yellow-500/5 px-4 py-3 text-sm">
      <div className="flex items-center gap-2 mb-2">
        <AlertTriangle className="h-4 w-4 text-yellow-400" />
        <span className="font-medium text-yellow-300/90">Confirmation Required</span>
      </div>

      <p className="text-white/70 text-xs mb-3">{request.description}</p>

      {status === "pending" && (
        <div className="flex items-center gap-2">
          <button
            onClick={handleConfirm}
            className="px-3 py-1 rounded bg-red-500/80 hover:bg-red-500 text-white text-xs font-medium transition-colors"
          >
            Confirm
          </button>
          <button
            onClick={handleCancel}
            className="px-3 py-1 rounded bg-white/10 hover:bg-white/20 text-white/70 text-xs transition-colors"
          >
            Cancel
          </button>
          <span className="text-[10px] text-white/30 ml-auto">
            {remaining}s remaining
          </span>
        </div>
      )}

      {status === "confirmed" && (
        <div className="text-xs text-green-400/70">Confirmed — executing...</div>
      )}
      {status === "cancelled" && (
        <div className="text-xs text-white/40">Cancelled</div>
      )}
      {status === "expired" && (
        <div className="text-xs text-white/30">Expired — ask allAI to try again</div>
      )}
    </div>
  );
});
