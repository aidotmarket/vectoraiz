import { useCallback, useEffect, useRef, useState } from "react";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import {
  CheckCircle2,
  AlertTriangle,
  AlertCircle,
  Info,
  Bell,
  Send,
  Loader2,
} from "lucide-react";
import { notificationsApi, diagnosticsApi, type ApiNotification } from "@/lib/api";

const typeConfig: Record<string, { icon: typeof Info; color: string }> = {
  success: { icon: CheckCircle2, color: "text-green-500" },
  error: { icon: AlertCircle, color: "text-red-500" },
  warning: { icon: AlertTriangle, color: "text-amber-500" },
  info: { icon: Info, color: "text-blue-500" },
  action_required: { icon: Bell, color: "text-purple-500" },
};

function timeAgo(dateStr: string): string {
  // Backend stores UTC timestamps — ensure they're parsed as UTC
  const utcStr = dateStr.endsWith("Z") || dateStr.includes("+") ? dateStr : dateStr + "Z";
  const diff = Date.now() - new Date(utcStr).getTime();
  if (diff < 0) return "just now";
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/** Check if a notification is a diagnostic transmission action */
function isDiagnosticAction(n: ApiNotification): boolean {
  if (n.type !== "action_required" || n.category !== "diagnostic") return false;
  try {
    const meta = n.metadata_json ? JSON.parse(n.metadata_json) : null;
    return meta?.action === "transmit_diagnostic";
  } catch {
    return false;
  }
}

interface NotificationSheetProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  countError?: boolean;
  onRefreshUnreadCount?: () => Promise<void>;
}

export default function NotificationSheet({
  open,
  onOpenChange,
  countError: bellCountError,
  onRefreshUnreadCount,
}: NotificationSheetProps) {
  const [notifications, setNotifications] = useState<ApiNotification[]>([]);
  const [loading, setLoading] = useState(false);
  const [hasLoaded, setHasLoaded] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [readError, setReadError] = useState(false);
  const [localCountError, setLocalCountError] = useState(false);
  const countError = bellCountError ?? localCountError;
  const [reading, setReading] = useState(false);
  const [transmitting, setTransmitting] = useState<string | null>(null);
  const mounted = useRef(false);
  const listRequest = useRef(0);
  const readPending = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const fetchNotifications = useCallback(async () => {
    const request = ++listRequest.current;
    setLoading(true);
    try {
      const res = await notificationsApi.list({ limit: 50 });
      if (!mounted.current || request !== listRequest.current) return;
      setNotifications(res.notifications);
      setHasLoaded(true);
      setLoadError(false);
    } catch {
      if (mounted.current && request === listRequest.current) setLoadError(true);
    } finally {
      if (mounted.current && request === listRequest.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) fetchNotifications();
    else ++listRequest.current;
  }, [open, fetchNotifications]);

  const refreshUnreadCount = async () => {
    try {
      // The Bell owns ordering between polling and mutation count refreshes.
      if (onRefreshUnreadCount) await onRefreshUnreadCount();
      else await notificationsApi.unreadCount();
      if (!mounted.current) return;
      if (bellCountError === undefined) setLocalCountError(false);
    } catch {
      if (!mounted.current) return;
      if (bellCountError === undefined) setLocalCountError(true);
    }
  };

  const markRead = async (id?: string) => {
    if (readPending.current) return;
    readPending.current = true;
    setReading(true);
    try {
      if (id) await notificationsApi.markRead(id);
      else await notificationsApi.markAllRead();
      if (!mounted.current) return;
      // A pre-mutation list response must not restore unread records.
      ++listRequest.current;
      setLoading(false);
      setNotifications((prev) => prev.map((n) => !id || n.id === id ? { ...n, read: true } : n));
      setReadError(false);
      await refreshUnreadCount();
    } catch {
      if (mounted.current) setReadError(true);
    } finally {
      readPending.current = false;
      if (mounted.current) setReading(false);
    }
  };

  const handleTransmitDiagnostic = async (notificationId: string) => {
    setTransmitting(notificationId);
    try {
      await diagnosticsApi.transmit();
      // Mark the action notification as read
      await notificationsApi.markRead(notificationId);
      if (!mounted.current) return;
      ++listRequest.current;
      setNotifications((prev) =>
        prev.map((n) => (n.id === notificationId ? { ...n, read: true } : n))
      );
      await refreshUnreadCount();
      // Refresh to show the new success notification
      await fetchNotifications();
    } catch (err) {
      // Show inline error — don't swallow
      const msg = err instanceof Error ? err.message : "Transmission failed";
      alert(msg);
    } finally {
      if (mounted.current) setTransmitting(null);
    }
  };

  const unreadCount = notifications.filter((n) => !n.read).length;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-[400px] sm:w-[440px] flex flex-col">
        <SheetHeader className="flex flex-row items-center justify-between pr-6">
          <SheetTitle>Notifications</SheetTitle>
          {unreadCount > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="text-xs text-muted-foreground"
              disabled={reading || transmitting !== null}
              onClick={() => markRead()}
            >
              Mark all read
            </Button>
          )}
        </SheetHeader>

        <div className="flex-1 overflow-y-auto mt-4 -mx-6 px-6">
          {loadError && (
            <div role="alert" className="text-sm text-destructive py-3">
              {hasLoaded
                ? "Could not refresh notifications. Showing stale results."
                : "Could not load notifications."}
              <Button variant="outline" size="sm" className="ml-2" disabled={loading || reading} onClick={fetchNotifications}>
                Retry
              </Button>
            </div>
          )}
          {readError && (
            <p role="alert" className="text-sm text-destructive py-3">
              Could not mark notifications as read. Please try the read action again.
            </p>
          )}
          {countError && (
            <div role="alert" className="text-sm text-destructive py-3">
              The unread count could not be refreshed.
              <Button variant="outline" size="sm" className="ml-2" disabled={reading} onClick={refreshUnreadCount}>
                Retry
              </Button>
            </div>
          )}
          {loading && notifications.length === 0 && (
            <div className="text-sm text-muted-foreground text-center py-8">
              Loading...
            </div>
          )}

          {!loading && hasLoaded && !loadError && notifications.length === 0 && (
            <div className="text-sm text-muted-foreground text-center py-8">
              No notifications yet
            </div>
          )}

          <div className="space-y-1">
            {notifications.map((n) => {
              const cfg = typeConfig[n.type] || typeConfig.info;
              const Icon = cfg.icon;
              const showTransmitButton = isDiagnosticAction(n) && !n.read;
              const isTransmitting = transmitting === n.id;

              return (
                <div key={n.id}>
                  <button
                    className={`w-full text-left p-3 rounded-lg transition-colors cursor-pointer ${
                      n.read
                        ? "opacity-60 hover:bg-secondary/50"
                        : "bg-secondary/50 hover:bg-secondary"
                    }`}
                    disabled={reading || transmitting !== null}
                    onClick={() => !n.read && !showTransmitButton && markRead(n.id)}
                  >
                    <div className="flex gap-3">
                      <Icon className={`w-4 h-4 mt-0.5 shrink-0 ${cfg.color}`} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium truncate">
                            {n.title}
                          </span>
                          {!n.read && (
                            <span className="w-2 h-2 rounded-full bg-blue-500 shrink-0" />
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">
                          {n.message}
                        </p>
                        <span className="text-[10px] text-muted-foreground mt-1 block">
                          {timeAgo(n.created_at)}
                        </span>
                      </div>
                    </div>
                  </button>

                  {showTransmitButton && (
                    <div className="ml-10 mt-1 mb-2">
                      <Button
                        size="sm"
                        variant="default"
                        className="gap-1.5 text-xs"
                        disabled={isTransmitting}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleTransmitDiagnostic(n.id);
                        }}
                      >
                        {isTransmitting ? (
                          <Loader2 className="w-3 h-3 animate-spin" />
                        ) : (
                          <Send className="w-3 h-3" />
                        )}
                        {isTransmitting ? "Sending..." : "Send Diagnostic to ai.market Support"}
                      </Button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}
