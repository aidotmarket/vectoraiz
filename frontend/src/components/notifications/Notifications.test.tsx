import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiNotification } from "@/lib/api";

const api = vi.hoisted(() => ({
  list: vi.fn(), unreadCount: vi.fn(), markRead: vi.fn(), markAllRead: vi.fn(),
  transmit: vi.fn(), network: vi.fn(() => { throw new Error("Unexpected HTTP"); }),
}));
vi.mock("@/lib/api", () => ({ notificationsApi: api, diagnosticsApi: { transmit: api.transmit } }));
vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ open, children }: { open: boolean; children: React.ReactNode }) => open ? <div>{children}</div> : null,
  SheetContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SheetHeader: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: React.ReactNode }) => <h2>{children}</h2>,
}));
vi.mock("@/components/ui/tooltip", () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
import NotificationSheet from "./NotificationSheet";
import NotificationBell from "./NotificationBell";

const row = (id = "notice"): ApiNotification => ({
  id, title: `Notice ${id}`, message: "Synthetic notification", read: false,
  type: "info", category: "system", metadata_json: null, batch_id: null,
  source: "test", created_at: "2026-10-15T00:00:00Z",
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function sheet() {
  const count = vi.fn();
  const props = { open: true, onOpenChange: vi.fn(), onRefreshUnreadCount: async () => {
    try {
      const res = await api.unreadCount();
      count(res.count);
      return true;
    } catch {
      count(null);
      return false;
    }
  } };
  const view = render(<NotificationSheet {...props} />);
  return { ...view, count, reopen: () => {
    view.rerender(<NotificationSheet {...props} open={false} />);
    view.rerender(<NotificationSheet {...props} />);
  } };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("fetch", api.network);
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(api.network);
  api.list.mockResolvedValue({ notifications: [row()], count: 1 });
  api.unreadCount.mockResolvedValue({ count: 74 });
  api.markRead.mockResolvedValue({});
  api.markAllRead.mockResolvedValue({ marked: 75 });
});
afterEach(() => {
  cleanup();
  expect(api.transmit).not.toHaveBeenCalled();
  expect(api.network).not.toHaveBeenCalled();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("notification read feedback", () => {
  it("distinguishes initial refusal from empty and retries successfully", async () => {
    api.list.mockRejectedValueOnce(new Error("Offline"));
    sheet();
    expect(await screen.findByRole("alert")).toHaveTextContent(/could not load notifications/i);
    expect(screen.queryByText("No notifications yet")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Notice notice")).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(api.list).toHaveBeenLastCalledWith({ limit: 50 });
  });
  it("shows the empty state only after a successful empty response", async () => {
    api.list.mockResolvedValue({ notifications: [], count: 0 });
    sheet();
    expect(await screen.findByText("No notifications yet")).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
  it("preserves rows and labels them stale after failed reopen, then recovers", async () => {
    const view = sheet();
    await screen.findByText("Notice notice");
    api.list.mockRejectedValueOnce(new Error("Offline"));
    view.reopen();
    expect(await screen.findByRole("alert")).toHaveTextContent(/stale/i);
    expect(screen.getByText("Notice notice")).toBeVisible();
    api.list.mockResolvedValueOnce({ notifications: [], count: 0 });
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("No notifications yet")).toBeVisible();
    expect(screen.queryByText("Notice notice")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
  it("uses the authoritative 74 total after reading one of 50 rows from 75 unread", async () => {
    api.list.mockResolvedValue({ notifications: Array.from({ length: 50 }, (_, i) => row(String(i))), count: 50 });
    const view = sheet();
    fireEvent.click(await screen.findByRole("button", { name: /Notice 0 / }));
    await waitFor(() => expect(view.count).toHaveBeenLastCalledWith(74));
    expect(api.markRead).toHaveBeenCalledWith("0");
    expect(view.count).not.toHaveBeenCalledWith(49);
  });
  it.each(["one", "all"])("keeps unread rows on rejected %s mutation and recovers on retry", async (mode) => {
    const mutation = mode === "one" ? api.markRead : api.markAllRead;
    mutation.mockRejectedValueOnce(new Error("Write refused"));
    const view = sheet();
    await screen.findByText("Notice notice");
    const click = () => fireEvent.click(screen.getByRole("button", { name: mode === "one" ? /Notice notice/ : "Mark all read" }));
    click();
    expect(await screen.findByRole("alert")).toHaveTextContent(/could not mark/i);
    expect(screen.getByRole("button", { name: "Mark all read" })).toBeVisible();
    expect(view.count).not.toHaveBeenCalled();
    expect(api.unreadCount).not.toHaveBeenCalled();
    click();
    await waitFor(() => expect(view.count).toHaveBeenLastCalledWith(74));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
  it.each(["one", "all"])("does not invent zero when count refresh after %s mutation fails", async (mode) => {
    api.unreadCount.mockRejectedValueOnce(new Error("Count refused"));
    const view = sheet();
    fireEvent.click(await screen.findByRole("button", { name: mode === "one" ? /Notice notice/ : "Mark all read" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/unread count/i);
    expect(view.count).not.toHaveBeenCalledWith(0);
    expect(view.count).toHaveBeenLastCalledWith(null);
    expect(screen.queryByRole("button", { name: "Mark all read" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(view.count).toHaveBeenLastCalledWith(74));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(mode === "one" ? api.markRead : api.markAllRead).toHaveBeenCalledTimes(1);
  });
  it("ignores a list response from an earlier open", async () => {
    const old = deferred<{ notifications: ApiNotification[]; count: number }>();
    api.list.mockReturnValueOnce(old.promise);
    const view = sheet();
    view.reopen();
    await screen.findByText("Notice notice");
    await act(async () => old.resolve({ notifications: [row("obsolete")], count: 1 }));
    expect(screen.queryByText("Notice obsolete")).not.toBeInTheDocument();
    expect(screen.getByText("Notice notice")).toBeVisible();
  });
  it("does not let a pending refresh restore a row after a successful read", async () => {
    const view = sheet();
    await screen.findByText("Notice notice");
    const old = deferred<{ notifications: ApiNotification[]; count: number }>();
    api.list.mockReturnValueOnce(old.promise);
    view.reopen();
    fireEvent.click(screen.getByRole("button", { name: /Notice notice/ }));
    await waitFor(() => expect(view.count).toHaveBeenLastCalledWith(74));
    await act(async () => old.resolve({ notifications: [row()], count: 1 }));
    expect(screen.queryByRole("button", { name: "Mark all read" })).not.toBeInTheDocument();
  });
  it("does not publish a pending mutation result after unmount", async () => {
    const pending = deferred<object>();
    api.markRead.mockReturnValueOnce(pending.promise);
    const view = sheet();
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    view.unmount();
    await act(async () => pending.resolve({}));
    expect(view.count).not.toHaveBeenCalled();
    expect(api.unreadCount).not.toHaveBeenCalled();
  });
  it("keeps a pending read unread and blocks duplicate writes", async () => {
    const pending = deferred<object>();
    api.markRead.mockReturnValueOnce(pending.promise);
    sheet();
    const notice = await screen.findByRole("button", { name: /Notice notice/ });
    fireEvent.click(notice);
    fireEvent.click(notice);
    expect(screen.getByRole("button", { name: "Mark all read" })).toBeDisabled();
    expect(api.markRead).toHaveBeenCalledTimes(1);
    expect(api.unreadCount).not.toHaveBeenCalled();
    await act(async () => pending.reject(new Error("Write refused")));
    expect(screen.getByRole("alert")).toHaveTextContent(/could not mark/i);
    expect(screen.getByRole("button", { name: "Mark all read" })).toBeEnabled();
  });
  it("refreshes the authoritative endpoint when used without a Bell", async () => {
    render(<NotificationSheet open onOpenChange={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    await waitFor(() => expect(api.unreadCount).toHaveBeenCalledTimes(1));
  });
  it("keeps diagnostic actions inert through ordinary reads and retry", async () => {
    api.list.mockResolvedValue({ notifications: [{ ...row(), type: "action_required", category: "diagnostic", metadata_json: '{"action":"transmit_diagnostic"}' }], count: 1 });
    sheet();
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    expect(api.markRead).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Send Diagnostic to ai.market Support" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Mark all read" }));
    await waitFor(() => expect(api.unreadCount).toHaveBeenCalled());
  });
});

describe("notification bell count", () => {
  it("shows 74 global unread after reading one of 50 rows from 75", async () => {
    api.list.mockResolvedValue({ notifications: Array.from({ length: 50 }, (_, i) => row(String(i))), count: 50 });
    api.unreadCount.mockResolvedValueOnce({ count: 75 }).mockResolvedValueOnce({ count: 75 }).mockResolvedValue({ count: 74 });
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 75 unread" }));
    fireEvent.click(await screen.findByRole("button", { name: /Notice 0 / }));
    expect(await screen.findByRole("button", { name: "Notifications, 74 unread" })).toBeVisible();
    expect(api.unreadCount).toHaveBeenCalledTimes(3);
  });
  it("uses the global total after read-all, including newly arrived unread notifications", async () => {
    api.unreadCount.mockResolvedValueOnce({ count: 75 }).mockResolvedValueOnce({ count: 75 }).mockResolvedValue({ count: 2 });
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 75 unread" }));
    fireEvent.click(await screen.findByRole("button", { name: "Mark all read" }));
    expect(await screen.findByRole("button", { name: "Notifications, 2 unread" })).toBeVisible();
  });
  it("distinguishes unknown from known zero and recovers on reopen", async () => {
    api.unreadCount.mockRejectedValueOnce(new Error("Count refused")).mockResolvedValue({ count: 0 });
    render(<NotificationBell />);
    const bell = screen.getByRole("button", { name: /notifications.*unavailable/i });
    await waitFor(() => expect(api.unreadCount).toHaveBeenCalledTimes(1));
    fireEvent.click(bell);
    await waitFor(() => expect(screen.getByRole("button", { name: "Notifications, 0 unread" })).toBeVisible());
    expect(screen.queryByText("?")).not.toBeInTheDocument();
  });
  it("recovers a failed initial count on the existing poll", async () => {
    vi.useFakeTimers();
    api.unreadCount.mockRejectedValueOnce(new Error("Count refused")).mockResolvedValue({ count: 75 });
    render(<NotificationBell />);
    await act(async () => {});
    expect(screen.getByRole("button", { name: /notifications.*unavailable/i })).toBeVisible();
    await act(async () => vi.advanceTimersByTime(30_000));
    expect(screen.getByRole("button", { name: "Notifications, 75 unread" })).toBeVisible();
  });
  it("preserves the last count with stale feedback on count failure after a read", async () => {
    api.unreadCount.mockResolvedValueOnce({ count: 75 }).mockResolvedValueOnce({ count: 75 }).mockRejectedValueOnce(new Error("Count refused"));
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 75 unread" }));
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    await screen.findByRole("alert");
    expect(screen.getByRole("button", { name: /75.*stale/i })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: "Notifications, 74 unread" })).toBeVisible();
  });
  it("keeps an unknown count unknown when a read succeeds but the count is refused", async () => {
    api.unreadCount.mockRejectedValue(new Error("Count refused"));
    render(<NotificationBell />);
    fireEvent.click(screen.getByRole("button", { name: /notifications.*unavailable/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    await screen.findByRole("alert");
    expect(screen.getByRole("button", { name: /notifications.*unavailable/i })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Notifications, 0 unread" })).not.toBeInTheDocument();
  });
  it("ignores a pre-read count request that finishes after the authoritative mutation count", async () => {
    const old = deferred<{ count: number }>();
    api.unreadCount.mockResolvedValueOnce({ count: 75 }).mockReturnValueOnce(old.promise).mockResolvedValue({ count: 74 });
    render(<NotificationBell />);
    fireEvent.click(await screen.findByRole("button", { name: "Notifications, 75 unread" }));
    fireEvent.click(await screen.findByRole("button", { name: /Notice notice/ }));
    await screen.findByRole("button", { name: "Notifications, 74 unread" });
    await act(async () => old.resolve({ count: 75 }));
    expect(screen.getByRole("button", { name: "Notifications, 74 unread" })).toBeVisible();
  });
  it("does not let a slow postmutation count overwrite a newer poll", async () => {
    vi.useFakeTimers();
    const slow = deferred<{ count: number }>();
    api.unreadCount.mockResolvedValueOnce({ count: 75 }).mockResolvedValueOnce({ count: 75 }).mockReturnValueOnce(slow.promise).mockResolvedValue({ count: 73 });
    render(<NotificationBell />);
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "Notifications, 75 unread" }));
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: /Notice notice/ }));
    await act(async () => {});
    await act(async () => vi.advanceTimersByTime(30_000));
    expect(screen.getByRole("button", { name: "Notifications, 73 unread" })).toBeVisible();
    await act(async () => slow.resolve({ count: 74 }));
    expect(screen.getByRole("button", { name: "Notifications, 73 unread" })).toBeVisible();
  });
});
