import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { RawFile } from "@/lib/api";

const h = vi.hoisted(() => ({
  info: vi.fn<(...args: unknown[]) => Promise<{ ok: boolean; json: () => Promise<unknown> }>>(),
  read: vi.fn<(id: string) => Promise<RawFile>>(),
  deny: vi.fn(() => { throw Error("Unexpected operation"); }),
  marketplace: vi.fn(),
  socket: { connectionStatus: "connected", allieAvailable: true },
}));
vi.mock("@/contexts/CoPilotContext", async () => {
  const { useMode } = await import("@/contexts/ModeContext");
  return { useCoPilot: () => ({ ...h.socket, isStandalone: useMode().isStandalone }) };
});
vi.mock("@/contexts/BrandContext", () => ({ useBrand: () => ({ name: "Owned brand", externalUrl: "#", logoPath: "data:,", sidebarLogoAlt: "Fixture" }) }));
vi.mock("@/hooks/use-toast", () => ({ toast: h.deny }));
vi.mock("@/lib/data-requests-api", () => ({ setMarketplaceApiUrl: h.marketplace }));
vi.mock("@/lib/api", () => ({
  getApiUrl: () => "http://owned.invalid",
  rawFilesApi: { getRawFile: (id: string) => h.read(id), getFileObjectUrl: h.deny, downloadRawFile: h.deny, updateRawFile: h.deny, deleteRawFile: h.deny },
}));

import { ModeProvider, useMode } from "./ModeContext";
import Sidebar from "@/components/layout/Sidebar";
import AiMarketPage from "@/pages/AiMarketPage";
import RawFileDetail from "@/pages/RawFileDetail";

const file: RawFile = {
  id: "owned-raw", filename: "owned.bin", file_size_bytes: 128, mime_type: "application/octet-stream",
  listing_status: null, price_cents: null, file_path: "fixture", content_hash: "fixture",
  created_at: "2026-01-01", updated_at: "2026-01-01",
  metadata: { title: "Owned", description: "Fixture", tags: ["owned"] },
};
const valid = (data: unknown) => ({ ok: true, json: async () => data });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
let observed: ReturnType<typeof useMode>;
function Observation() {
  observed = useMode();
  return <output data-testid="read-status">{observed.readStatus}</output>;
}
function surfaces() {
  return <MemoryRouter initialEntries={["/raw-files/owned-raw"]}>
    <Sidebar collapsed={false} onToggle={h.deny} /><AiMarketPage />
    <Routes><Route path="/raw-files/:id" element={<RawFileDetail />} /></Routes><Observation />
  </MemoryRouter>;
}
const mount = () => render(<ModeProvider>{surfaces()}</ModeProvider>);
async function status(view: ReturnType<typeof render>, value: string) {
  await waitFor(() => expect(view.getByTestId("read-status").textContent).toBe(value));
  await waitFor(() => expect(view.getByText("Listing Readiness")).toBeTruthy());
}
function noConfirmation(view: ReturnType<typeof render>) {
  const text = view.container.textContent;
  for (const copy of ["Standalone Mode", "Connected mode configured", "Configured for ai.market", "allAI is active", "./start.sh --setup-allie", "Connect to ai.market to access", "Coming Soon"]) {
    expect(text).not.toContain(copy);
  }
}
function fallback() {
  expect(observed.mode).toBe("standalone");
  expect(observed.isStandalone).toBe(true);
  expect(observed.isConnected).toBe(false);
  expect(observed.channel).toBe("direct");
  expect(observed.version).toBe("0.0.0");
  expect(observed.features).toEqual({ allai: false, marketplace: false, earnings: false, local_auth: true });
  expect(observed.hasFeature("local_auth")).toBe(true);
}
function rawControls(view: ReturnType<typeof render>) {
  return ["Download", "Delete", "Save Metadata", "Create Listing", "Vectorize This File"].map(name => {
    const button = view.getByRole("button", { name }) as HTMLButtonElement;
    return [name, button.disabled];
  });
}
beforeEach(() => {
  h.info.mockReset(); h.read.mockReset().mockResolvedValue(file); h.deny.mockClear(); h.marketplace.mockReset();
  h.socket.connectionStatus = "connected"; h.socket.allieAvailable = true;
  vi.stubGlobal("fetch", (url: unknown, ...args: unknown[]) => {
    if (url !== "http://owned.invalid/api/system/info" || args.length) return h.deny();
    return h.info(url);
  });
  vi.stubGlobal("XMLHttpRequest", h.deny); vi.stubGlobal("WebSocket", h.deny);
  vi.stubGlobal("crypto", { randomUUID: () => "owned-mode" });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); expect(h.deny).not.toHaveBeenCalled(); });

it.each(["http", "transport", "json", "missing", "invalid", "null", "array"])("keeps startup %s failure unknown and fallback/actions intact", async failure => {
  if (failure === "http") h.info.mockResolvedValue({ ok: false, json: async () => ({}) });
  else if (failure === "transport") h.info.mockRejectedValue(Error("private transport error"));
  else if (failure === "json") h.info.mockResolvedValue({ ok: true, json: async () => { throw Error("private parse error"); } });
  else h.info.mockResolvedValue(valid(failure === "missing" ? {} : failure === "null" ? null : failure === "array" ? [] : { mode: "invalid", features: { marketplace: true }, marketplace_api_url: "http://rejected.invalid" }));
  const view = mount(); await status(view, "failed"); noConfirmation(view); fallback();
  expect(view.container.textContent).toContain("configuration is unavailable");
  expect(view.container.textContent).not.toContain("private");
  expect(rawControls(view).every(([, disabled]) => disabled === false)).toBe(true);
  expect(view.queryByRole("link", { name: "Earnings" })).toBeNull();
  expect(h.read).toHaveBeenCalledExactlyOnceWith("owned-raw");
  expect(h.info).toHaveBeenCalledTimes(1); expect(h.marketplace).not.toHaveBeenCalled();
});
it.each(["standalone", "connected"])("confirms minimal %s without optional metadata", async mode => {
  h.info.mockResolvedValue(valid({ mode })); const view = mount(); await status(view, "confirmed");
  expect(observed.mode).toBe(mode); expect(observed.channel).toBe("direct"); expect(observed.version).toBe("0.0.0");
  expect(observed.hasFeature("local_auth")).toBe(true); expect(observed.hasFeature("marketplace")).toBe(false);
  expect(view.container.textContent).toContain(mode === "standalone" ? "Standalone Mode" : "Connected mode configured");
  expect(view.container.textContent?.includes("./start.sh --setup-allie")).toBe(mode === "standalone");
  expect(view.queryByRole("link", { name: /View Earnings Dashboard/ })).toBeNull();
  expect(view.queryByRole("link", { name: "Earnings" })).toBeNull();
  expect(view.container.textContent).toContain("Coming Soon");
  if (mode === "connected") {
    expect(view.container.textContent).toContain("The data marketplace is not enabled for this configuration.");
    expect(view.container.textContent).not.toContain("Connect to ai.market to access the data marketplace");
  } else {
    expect(view.container.textContent).toContain("Connect to ai.market to access the data marketplace");
  }
  expect(h.marketplace).toHaveBeenCalledExactlyOnceWith(null);
});
it("keeps loading truthful and coalesces attempts during the initial read", async () => {
  const pending = deferred<ReturnType<typeof valid>>(); h.info.mockReturnValue(pending.promise);
  const view = mount(); await status(view, "pending"); noConfirmation(view); fallback();
  expect((view.getByRole("button", { name: "Retry configuration" }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { await observed.retryConfiguration(); await observed.retryConfiguration(); });
  expect(h.info).toHaveBeenCalledTimes(1);
  await act(async () => pending.resolve(valid({ mode: "standalone" }))); await status(view, "confirmed");
});
it("retries manually through pending, failure and recovery across all surfaces without raw refetch", async () => {
  h.info.mockRejectedValueOnce(Error("offline")); const view = mount(); await status(view, "failed");
  const controls = rawControls(view); const pending = deferred<ReturnType<typeof valid>>(); h.info.mockReturnValueOnce(pending.promise);
  fireEvent.click(view.getAllByRole("button", { name: "Retry configuration" })[0]);
  await status(view, "pending"); noConfirmation(view);
  await act(async () => { await observed.retryConfiguration(); await observed.retryConfiguration(); });
  expect(h.info).toHaveBeenCalledTimes(2);
  await act(async () => pending.reject(Error("still offline"))); await status(view, "failed"); noConfirmation(view);
  h.info.mockResolvedValueOnce(valid({ mode: "standalone" }));
  fireEvent.click(view.getAllByRole("button", { name: "Retry configuration" }).at(-1)!);
  await status(view, "confirmed"); expect(view.container.textContent).toContain("Standalone Mode");
  expect(view.container.textContent).toContain("./start.sh --setup-allie");
  expect(h.info).toHaveBeenCalledTimes(3); expect(h.read).toHaveBeenCalledTimes(1); expect(rawControls(view)).toEqual(controls);
});
it.each(["marketplace", "aim-data", "unknown"])("retains %s channel/feature/access observations and qualified links after a failed refresh", async channel => {
  h.info.mockResolvedValueOnce(valid({ mode: "connected", channel, version: "owned-version", features: { allai: true, marketplace: true, earnings: true, local_auth: false }, marketplace_api_url: "http://owned-market.invalid" }));
  const view = mount(); await status(view, "confirmed");
  const navigation = () => Array.from(view.container.querySelectorAll("aside a")).map(a => a.getAttribute("href"));
  const links = navigation(); const controls = rawControls(view); const features = observed.features;
  expect(view.getByRole("link", { name: /View Earnings Dashboard/ }).getAttribute("href")).toBe("/earnings");
  const pending = deferred<ReturnType<typeof valid>>(); h.info.mockReturnValueOnce(pending.promise);
  fireEvent.click(view.getByRole("button", { name: "Retry configuration" })); await status(view, "pending"); noConfirmation(view);
  expect(view.container.textContent).toContain("last known configuration");
  await act(async () => pending.resolve(valid({ mode: "bad", channel: "direct", features: {} })));
  await status(view, "failed"); noConfirmation(view);
  expect(observed.mode).toBe("connected"); expect(observed.features).toBe(features); expect(observed.hasFeature("local_auth")).toBe(false);
  expect(observed.channel).toBe(channel === "unknown" ? "direct" : channel); expect(observed.version).toBe("owned-version");
  expect(navigation()).toEqual(links); expect(rawControls(view)).toEqual(controls);
  expect(view.getByRole("link", { name: /View Earnings Dashboard/ }).getAttribute("href")).toBe("/earnings");
  expect(h.marketplace).toHaveBeenCalledExactlyOnceWith("http://owned-market.invalid");
  h.info.mockResolvedValueOnce(valid({ mode: "connected" }));
  await act(async () => observed.retryConfiguration()); await status(view, "confirmed");
  expect(view.container.textContent).toContain("Connected mode configured"); expect(view.container.textContent).toContain("allAI is active");
  expect(observed.hasFeature("marketplace")).toBe(false); expect(observed.channel).toBe("direct"); expect(h.marketplace).toHaveBeenLastCalledWith(null);
});
it("ignores an unmounted provider completion and retired retry callback", async () => {
  const old = deferred<ReturnType<typeof valid>>(); h.info.mockReturnValueOnce(old.promise);
  const first = mount(); const retry = observed.retryConfiguration; first.unmount();
  h.info.mockResolvedValueOnce(valid({ mode: "connected", marketplace_api_url: "http://current.invalid" }));
  const current = mount(); await status(current, "confirmed");
  await act(async () => { old.resolve(valid({ mode: "standalone", marketplace_api_url: "http://stale.invalid" })); await retry(); });
  expect(observed.mode).toBe("connected"); expect(h.info).toHaveBeenCalledTimes(2);
  expect(h.marketplace).toHaveBeenCalledExactlyOnceWith("http://current.invalid");
});
it.each(["success", "failure"])("ignores a stale StrictMode read %s while the current read owns status and side effects", async outcome => {
  const stale = deferred<ReturnType<typeof valid>>(); const current = deferred<ReturnType<typeof valid>>();
  h.info.mockReturnValueOnce(stale.promise).mockReturnValueOnce(current.promise);
  const view = render(<StrictMode><ModeProvider><Observation /></ModeProvider></StrictMode>);
  expect(h.info).toHaveBeenCalledTimes(2);
  await act(async () => { if (outcome === "success") stale.resolve(valid({ mode: "standalone", marketplace_api_url: "http://stale.invalid" })); else stale.reject(Error("stale")); });
  expect(view.getByTestId("read-status").textContent).toBe("pending"); expect(h.marketplace).not.toHaveBeenCalled();
  await act(async () => { await observed.retryConfiguration(); }); expect(h.info).toHaveBeenCalledTimes(2);
  await act(async () => current.resolve(valid({ mode: "connected", marketplace_api_url: "http://current.invalid" })));
  expect(view.getByTestId("read-status").textContent).toBe("confirmed"); expect(observed.mode).toBe("connected");
  expect(h.marketplace).toHaveBeenCalledExactlyOnceWith("http://current.invalid");
});

it.each(["success", "failure"])("ignores stale JSON %s after a newer read confirms", async outcome => {
  const staleJson = deferred<unknown>(); const current = deferred<ReturnType<typeof valid>>();
  h.info.mockResolvedValueOnce({ ok: true, json: () => staleJson.promise }).mockReturnValueOnce(current.promise);
  const view = render(<StrictMode><ModeProvider><Observation /></ModeProvider></StrictMode>);
  await act(async () => current.resolve(valid({ mode: "connected", marketplace_api_url: "http://current.invalid" })));
  expect(view.getByTestId("read-status").textContent).toBe("confirmed");
  await act(async () => { if (outcome === "success") staleJson.resolve({ mode: "standalone", marketplace_api_url: "http://stale.invalid" }); else staleJson.reject(Error("stale JSON")); });
  expect(view.getByTestId("read-status").textContent).toBe("confirmed"); expect(observed.mode).toBe("connected");
  expect(h.marketplace).toHaveBeenCalledExactlyOnceWith("http://current.invalid");
});
