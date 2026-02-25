import { useCallback } from "react";
import { X, AlertTriangle, CheckCircle, Settings, Shield, Clock, AlertOctagon, Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export interface NudgeData {
  nudge_id: string;
  trigger: string;
  message: string;
  dismissable: boolean;
  icon?: string;
}

interface NudgeBannerProps {
  nudge: NudgeData;
  onDismiss: (nudgeId: string, trigger: string, permanent: boolean) => void;
  onAction?: (nudgeId: string, trigger: string) => void;
}

const ICON_MAP: Record<string, React.ComponentType<{ className?: string }>> = {
  AlertTriangle,
  CheckCircle,
  Settings,
  Shield,
  Clock,
  AlertOctagon,
  Info,
};

const TRIGGER_STYLES: Record<string, { bg: string; border: string; iconColor: string }> = {
  error_event: { bg: "bg-destructive/10", border: "border-destructive/30", iconColor: "text-destructive" },
  upload_complete: { bg: "bg-green-500/10", border: "border-green-500/30", iconColor: "text-green-600" },
  processing_complete: { bg: "bg-green-500/10", border: "border-green-500/30", iconColor: "text-green-600" },
  missing_config: { bg: "bg-amber-500/10", border: "border-amber-500/30", iconColor: "text-amber-600" },
  pii_detected: { bg: "bg-amber-500/10", border: "border-amber-500/30", iconColor: "text-amber-600" },
  long_running_op: { bg: "bg-blue-500/10", border: "border-blue-500/30", iconColor: "text-blue-600" },
  destructive_action: { bg: "bg-destructive/10", border: "border-destructive/30", iconColor: "text-destructive" },
};

export default function NudgeBanner({ nudge, onDismiss, onAction }: NudgeBannerProps) {
  const Icon = ICON_MAP[nudge.icon || "Info"] || Info;
  const styles = TRIGGER_STYLES[nudge.trigger] || { bg: "bg-muted/50", border: "border-border", iconColor: "text-muted-foreground" };
  const showDontShowAgain = nudge.trigger === "missing_config";

  const handleDismiss = useCallback(() => {
    onDismiss(nudge.nudge_id, nudge.trigger, false);
  }, [nudge, onDismiss]);

  const handlePermanentDismiss = useCallback(() => {
    onDismiss(nudge.nudge_id, nudge.trigger, true);
  }, [nudge, onDismiss]);

  const handleAction = useCallback(() => {
    onAction?.(nudge.nudge_id, nudge.trigger);
  }, [nudge, onAction]);

  return (
    <div className={cn("mx-3 my-2 rounded-lg border p-3", styles.bg, styles.border)}>
      <div className="flex items-start gap-2.5">
        <Icon className={cn("h-4 w-4 mt-0.5 flex-shrink-0", styles.iconColor)} />
        <div className="flex-1 min-w-0">
          <p className="text-sm text-foreground leading-relaxed">{nudge.message}</p>
          {showDontShowAgain && (
            <button
              onClick={handlePermanentDismiss}
              className="text-xs text-muted-foreground hover:text-foreground mt-1.5 underline underline-offset-2"
            >
              Don't show again
            </button>
          )}
          {onAction && (
            <Button
              variant="link"
              size="sm"
              onClick={handleAction}
              className="h-auto p-0 mt-1.5 text-xs"
            >
              Take action
            </Button>
          )}
        </div>
        {nudge.dismissable && (
          <button
            onClick={handleDismiss}
            className="text-muted-foreground hover:text-foreground transition-colors p-0.5 -mt-0.5 -mr-0.5"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
    </div>
  );
}
