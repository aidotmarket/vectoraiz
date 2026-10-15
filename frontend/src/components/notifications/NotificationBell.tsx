import { useEffect, useState, useCallback, useRef } from "react";
import { Bell } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { notificationsApi } from "@/lib/api";
import NotificationSheet from "./NotificationSheet";

const POLL_INTERVAL_MS = 30_000;

export default function NotificationBell() {
  const [unreadCount, setUnreadCount] = useState<number | null>(null);
  const [countError, setCountError] = useState(false);
  const [sheetOpen, setSheetOpen] = useState(false);
  const mounted = useRef(false);
  const countRequest = useRef(0);

  const fetchUnreadCount = useCallback(async () => {
    const request = ++countRequest.current;
    try {
      const res = await notificationsApi.unreadCount();
      if (!mounted.current || request !== countRequest.current) return;
      setUnreadCount(res.count);
      setCountError(false);
    } catch {
      if (!mounted.current || request !== countRequest.current) return;
      setCountError(true);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    fetchUnreadCount();
    const interval = setInterval(fetchUnreadCount, POLL_INTERVAL_MS);
    return () => {
      mounted.current = false;
      clearInterval(interval);
    };
  }, [fetchUnreadCount]);

  // Opening and closing both retry an unavailable count.
  const handleOpenChange = (open: boolean) => {
    setSheetOpen(open);
    fetchUnreadCount();
  };

  const handleRefreshUnreadCount = () => {
    // Keep the last known count while this global read is pending.
    return fetchUnreadCount();
  };
  const countLabel = unreadCount === null
    ? "Notifications, unread count unavailable"
    : countError
      ? `Notifications, ${unreadCount} unread (stale; count unavailable)`
      : `Notifications, ${unreadCount} unread`;

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            className="relative w-9 h-9"
            data-notification-bell
            aria-label={countLabel}
            onClick={() => handleOpenChange(true)}
          >
            <Bell className="w-4 h-4 text-muted-foreground" />
            {unreadCount !== null && unreadCount > 0 && (
              <span className="absolute -top-0.5 -right-0.5 min-w-[18px] h-[18px] rounded-full bg-red-500 text-white text-[10px] font-bold flex items-center justify-center px-1">
                {unreadCount > 99 ? "99+" : unreadCount}
              </span>
            )}
            {(unreadCount === null || countError) && (
              <span aria-hidden="true" className="absolute -bottom-0.5 -right-0.5 rounded-full bg-background px-0.5 text-xs font-bold text-amber-600">?</span>
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          {countLabel}
        </TooltipContent>
      </Tooltip>

      <NotificationSheet
        open={sheetOpen}
        onOpenChange={handleOpenChange}
        countError={countError}
        onRefreshUnreadCount={handleRefreshUnreadCount}
      />
    </>
  );
}
