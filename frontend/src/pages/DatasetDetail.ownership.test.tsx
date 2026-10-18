import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StrictMode, type ReactNode } from "react";
import DatasetDetail from "./DatasetDetail";
import { flushSync } from "react-dom";
import { toast } from "@/hooks/use-toast";

// Mount the actual page; isolate API, routes, contexts, preview/publish children
// and presentation. All tab contents stay visible for ownership assertions.
const { api, presentation, route, navigate, unpublish, modal } = vi.hoisted(() => ({
  route: { id: "A" as string | undefined }, navigate: vi.fn(), unpublish: vi.fn(),
  modal: { success: undefined as undefined | (() => void) },
  api: {
    get: vi.fn(), getSample: vi.fn(), getStatistics: vi.fn(), getReadiness: vi.fn(),
    delete: vi.fn(), confirm: vi.fn(),
  },
  presentation: async (names: string[]) => {
    const { createElement } = await import("react");
    return Object.fromEntries(names.map((name) => [name,
      ({ children }: { children?: ReactNode }) => createElement("div", null, children),
    ]));
  },
}));
vi.mock("react-router-dom", () => ({
  useParams: () => route, useNavigate: () => navigate,
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/contexts/MarketplaceContext", () => ({ useMarketplace: () => ({
  isPublished: () => false, getPublishedData: () => undefined, unpublishDataset: unpublish,
}) }));
vi.mock("@/lib/api", () => ({ datasetsApi: api }));
vi.mock("@/contexts/ModeContext", () => ({ useMode: () => ({ isStandalone: false, hasFeature: () => true }) }));
vi.mock("@/hooks/useChannel", () => ({ useChannel: () => "marketplace" }));
vi.mock("@/hooks/use-toast", () => ({ toast: vi.fn() }));
vi.mock("@/components/PublishModal", () => ({ default: ({ open, dataset, onPublishSuccess }: {
  open: boolean; dataset: { id: string }; onPublishSuccess: () => void;
}) => {
  modal.success = onPublishSuccess;
  return open ? <button onClick={onPublishSuccess}>Publish {dataset.id}</button> : null;
} }));
vi.mock("@/components/DatasetPreview", () => ({ default: () => null }));
vi.mock("@/components/ui/button", async () => ({
  Button: ({ children, onClick, disabled }: { children: ReactNode; onClick?: () => void; disabled?: boolean }) =>
    <button onClick={onClick} disabled={disabled}>{children}</button>,
}));
vi.mock("@/components/ui/card", () => presentation(["Card", "CardContent", "CardDescription", "CardHeader", "CardTitle"]));
vi.mock("@/components/ui/badge", () => presentation(["Badge"]));
vi.mock("@/components/ui/progress", () => presentation(["Progress"]));
vi.mock("@/components/ui/tabs", () => presentation(["Tabs", "TabsContent", "TabsList", "TabsTrigger"]));
vi.mock("@/components/ui/table", () => presentation(["Table", "TableBody", "TableCell", "TableHead", "TableHeader", "TableRow"]));
vi.mock("@/components/ui/scroll-area", () => presentation(["ScrollArea", "ScrollBar"]));
vi.mock("@/components/ui/skeleton", () => presentation(["Skeleton"]));

const record = (id: string) => ({
  id, original_filename: `${id}.csv`, status: "ready", file_type: "csv",
  metadata: { row_count: 30, column_count: 1, columns: [{ name: "value", type: "string" }] },
  created_at: "2026-10-17", updated_at: "2026-10-17",
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const read = () => deferred<ReturnType<typeof record>>();
const settle = async <T,>(pending: ReturnType<typeof deferred<T>>, value: T, fail = false) => {
  await act(async () => { if (fail) pending.reject(Error("Retired failure")); else pending.resolve(value); });
};
const heading = (id: string) => screen.findByRole("heading", { name: `${id}.csv` });
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const move = (view: ReturnType<typeof render>, id: string | undefined) => {
  route.id = id;
  view.rerender(<DatasetDetail />);
};
function expectPending() {
  expect(screen.queryByRole("heading")).toBeNull();
  for (const name of ["Delete", "Search", "Query", "Publish to ai.market"]) {
    expect(screen.queryByRole("button", { name })).toBeNull();
  }
  expect(screen.queryByRole("alert")).toBeNull();
}
beforeEach(() => {
  vi.resetAllMocks();
  route.id = "A";
  modal.success = undefined;
  vi.stubGlobal("fetch", vi.fn(() => { throw Error("Unexpected network"); }));
  vi.stubGlobal("XMLHttpRequest", vi.fn(() => { throw Error("Unexpected network"); }));
  vi.spyOn(window, "confirm").mockReturnValue(true);
  api.get.mockImplementation(async (id: string) => record(id));
  api.getSample.mockResolvedValue({ sample: [{ value: "sample-current" }] });
  api.getStatistics.mockResolvedValue({ statistics: [{ column: "stat-current", type: "string", unique_count: 4 }] });
  api.getReadiness.mockResolvedValue({ statistical_profile: { row_count: 987, column_count: 654, columns: [] } });
});
afterEach(() => {
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  expect(api.confirm).not.toHaveBeenCalled(); // Preview owns confirmation; this page never starts it.
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it.each([false, true])("ignores retired primary success/failure after B loads (failure=%s)", async (fail) => {
  const old = read();
  api.get.mockReturnValueOnce(old.promise);
  const view = render(<DatasetDetail />);
  move(view, "B");
  await heading("B");
  await settle(old, record("A"), fail);
  expect(await heading("B")).toBeTruthy();
  expect(screen.queryByText("Retired failure")).toBeNull();
  expect(api.getSample.mock.calls.map(([id]) => id)).toEqual(["B"]);
});
it.each([false, true])("keeps pending B hidden from retired primary/finally (failure=%s)", async (fail) => {
  const old = read(), next = read();
  api.get.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
  const view = render(<DatasetDetail />);
  move(view, "B");
  await settle(old, record("A"), fail);
  expectPending();
  await settle(next, record("B"));
  expect(await heading("B")).toBeTruthy();
});
it.each([false, true])("retires first A across A → B → A (failure=%s)", async (fail) => {
  const old = read(), latest = read();
  api.get.mockReturnValueOnce(old.promise).mockResolvedValueOnce(record("B")).mockReturnValueOnce(latest.promise);
  const view = render(<DatasetDetail />);
  move(view, "B");
  await heading("B");
  move(view, "A");
  await settle(old, record("A"), fail);
  expectPending();
  await settle(latest, { ...record("A"), original_filename: "Latest A.csv" });
  expect(await screen.findByRole("heading", { name: "Latest A.csv" })).toBeTruthy();
});
it.each([false, true])("ignores retired auxiliary results/failures and finally while B waits (failure=%s)", async (fail) => {
  const sample = deferred<{ sample: { value: string }[] }>();
  const stats = deferred<{ statistics: { column: string; type: string; unique_count: number }[] }>();
  const readiness = deferred<{ statistical_profile: { row_count: number; column_count: number; columns: [] } }>();
  const next = read();
  api.getReadiness.mockResolvedValue({});
  api.getSample.mockReturnValueOnce(sample.promise);
  api.getStatistics.mockReturnValueOnce(stats.promise);
  api.getReadiness.mockReturnValueOnce(readiness.promise);
  const view = render(<DatasetDetail />);
  await act(async () => {}); // A primary has launched all auxiliary reads.
  expect(api.getReadiness).toHaveBeenCalledWith("A");
  api.get.mockReturnValueOnce(next.promise);
  move(view, "B");
  await settle(sample, { sample: [{ value: "OLD-SAMPLE" }] }, fail);
  await settle(stats, { statistics: [{ column: "OLD-STAT", type: "string", unique_count: 99 }] }, fail);
  await settle(readiness, { statistical_profile: { row_count: 987, column_count: 654, columns: [] } }, fail);
  expectPending();
  await settle(next, record("B"));
  await heading("B");
  expect(document.body.textContent).not.toContain("OLD-");
  expect(document.body.textContent).not.toContain("987 rows, 654 columns");
  expect(screen.getByText("sample-current")).toBeTruthy();
  expect(screen.getByText("stat-current")).toBeTruthy();
});
it.each([false, true])("does not replace loaded B with late auxiliary data/errors (failure=%s)", async (fail) => {
  const sample = deferred<{ sample: { value: string }[] }>();
  api.getSample.mockReturnValueOnce(sample.promise);
  const view = render(<DatasetDetail />);
  await act(async () => {});
  move(view, "B");
  await heading("B");
  await settle(sample, { sample: [{ value: "OLD-SAMPLE" }] }, fail);
  expect(await heading("B")).toBeTruthy();
  expect(screen.getByText("sample-current")).toBeTruthy();
  expect(screen.queryByText("OLD-SAMPLE")).toBeNull();
});
it("clears loaded auxiliary, pagination and publish state when next visit auxiliary reads fail", async () => {
  const view = render(<DatasetDetail />);
  await heading("A");
  fireEvent.click(button("Next"));
  fireEvent.click(button("Publish to ai.market"));
  expect(button("Publish A")).toBeTruthy();
  api.getSample.mockRejectedValueOnce(Error("No sample"));
  api.getStatistics.mockRejectedValueOnce(Error("No stats"));
  api.getReadiness.mockRejectedValueOnce(Error("No readiness"));
  move(view, "B");
  await heading("B");
  expect(screen.queryByText("sample-current")).toBeNull();
  expect(screen.queryByText("stat-current")).toBeNull();
  expect(screen.queryByText("987 rows, 654 columns")).toBeNull();
  expect(button("Previous").disabled).toBe(true);
  expect(screen.queryByRole("button", { name: "Publish B" })).toBeNull();
});
it("rejects wrong returned identity before any auxiliary read or action", async () => {
  api.get.mockResolvedValueOnce(record("B"));
  render(<DatasetDetail />);
  expect((await screen.findByRole("alert")).textContent).toContain("does not match");
  expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  expect(api.getSample).not.toHaveBeenCalled();
  expect(api.delete).not.toHaveBeenCalled();
});
it("offers explicit retry for current read failures and Back for missing id without automatic retries", async () => {
  api.get.mockRejectedValueOnce(Error("Current read failed")).mockRejectedValueOnce(Error("Still failed"));
  const view = render(<DatasetDetail />);
  expect((await screen.findByRole("alert")).textContent).toBe("Current read failed");
  expect(api.get).toHaveBeenCalledTimes(1);
  fireEvent.click(button("Retry"));
  expect((await screen.findByRole("alert")).textContent).toBe("Still failed");
  fireEvent.click(button("Retry"));
  await heading("A");
  expect(api.get.mock.calls).toEqual([["A"], ["A"], ["A"]]);
  move(view, undefined);
  expect((await screen.findByRole("alert")).textContent).toBe("No dataset ID provided");
  expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  fireEvent.click(button("Back to Datasets"));
  expect(navigate).toHaveBeenCalledWith("/datasets");
  expect(api.get).toHaveBeenCalledTimes(3);
});
it.each([false, true])("retires delete view effects across A → B → A without releasing current busy (failure=%s)", async (fail) => {
  const old = deferred<void>(), current = deferred<void>();
  api.delete.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
  const view = render(<DatasetDetail />);
  await heading("A");
  const oldPublishSuccess = modal.success!;
  fireEvent.click(button("Delete"));
  move(view, "B");
  await heading("B");
  move(view, "A");
  await heading("A");
  expect(button("Delete").disabled).toBe(false);
  fireEvent.click(button("Delete"));
  await settle(old, undefined, fail);
  act(() => oldPublishSuccess());
  expect(button("Delete").disabled).toBe(true);
  expect(toast).not.toHaveBeenCalled();
  expect(navigate).not.toHaveBeenCalled();
  expect(api.delete.mock.calls).toEqual([["A"], ["A"]]);
  await settle(current, undefined);
  expect(toast).toHaveBeenCalledWith(expect.objectContaining({ title: "Dataset deleted" }));
  expect(navigate).toHaveBeenCalledWith("/datasets");
});
it.each(["primary", "auxiliary", "delete"] as const)("retires %s success/failure after unmount", async (operation) => {
  for (const fail of [false, true]) {
    const pending = read();
    if (operation === "primary") api.get.mockReturnValueOnce(pending.promise);
    if (operation === "auxiliary") api.getReadiness.mockReturnValueOnce(pending.promise);
    if (operation === "delete") api.delete.mockReturnValueOnce(pending.promise);
    const view = render(<DatasetDetail />);
    await act(async () => {});
    if (operation === "delete") { await heading("A"); fireEvent.click(button("Delete")); }
    view.unmount();
    await settle(pending, record("A"), fail);
    expect(toast).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
    if (operation === "primary") expect(api.getSample).not.toHaveBeenCalled();
  }
});
it("preserves ordinary load, sample, search/query, publish and delete confirmation/cancel/error/success", async () => {
  render(<DatasetDetail />);
  await heading("A");
  expect(api.delete).not.toHaveBeenCalled();
  expect(window.confirm).not.toHaveBeenCalled();
  expect(unpublish).not.toHaveBeenCalled();
  expect(toast).not.toHaveBeenCalled();
  expect(api.getSample).toHaveBeenCalledWith("A", 20);
  expect(screen.getByText("sample-current")).toBeTruthy();
  fireEvent.click(button("Search"));
  expect(navigate).toHaveBeenLastCalledWith("/search?dataset=A");
  fireEvent.click(button("Query"));
  expect(navigate).toHaveBeenLastCalledWith("/sql?dataset=A");
  fireEvent.click(button("Publish to ai.market"));
  fireEvent.click(button("Publish A"));
  expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ title: "Dataset published" }));
  vi.mocked(window.confirm).mockReturnValueOnce(false);
  fireEvent.click(button("Delete"));
  expect(api.delete).not.toHaveBeenCalled();
  expect(window.confirm).toHaveBeenCalledWith("Are you sure you want to delete this dataset? This action cannot be undone.");
  api.delete.mockRejectedValueOnce(Error("Delete failed"));
  fireEvent.click(button("Delete"));
  await act(async () => {});
  expect(button("Delete").disabled).toBe(false);
  expect(toast).toHaveBeenLastCalledWith(expect.objectContaining({ title: "Delete failed" }));
  fireEvent.click(button("Delete"));
  await act(async () => {});
  expect(api.delete).toHaveBeenLastCalledWith("A");
  expect(navigate).toHaveBeenLastCalledWith("/datasets");
});
it("loads normally in React strict effects without accepting the replayed read", async () => {
  const old = read();
  api.get.mockReturnValueOnce(old.promise);
  render(<StrictMode><DatasetDetail /></StrictMode>);
  await heading("A");
  await settle(old, { ...record("A"), original_filename: "Retired.csv" });
  expect(await heading("A")).toBeTruthy();
  expect(screen.queryByRole("heading", { name: "Retired.csv" })).toBeNull();
});

it("does not start deletion if the visit retires during the confirmation", async () => {
  const view = render(<DatasetDetail />);
  await heading("A");
  vi.mocked(window.confirm).mockImplementationOnce(() => {
    flushSync(() => move(view, "B"));
    return true;
  });
  fireEvent.click(button("Delete"));
  await heading("B");
  expect(api.delete).not.toHaveBeenCalled();
  expect(toast).not.toHaveBeenCalled();
  expect(button("Delete").disabled).toBe(false);
});
