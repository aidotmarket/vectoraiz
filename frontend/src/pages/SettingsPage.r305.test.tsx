import React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PortalSettingsSection } from "./SettingsPage";

const { notice } = vi.hoisted(() => ({ notice: vi.fn() }));
vi.mock("@/hooks/use-toast", () => ({ toast: notice }));
vi.mock("@/contexts/BrandContext", () => ({ useBrand: () => ({ name: "Owned fixture" }) }));
const config = { enabled: true, tier: "sso", base_url: "https://owned.invalid", active_session_count: 7, oidc_issuer: "https://owned.invalid", datasets: { owned: { portal_visible: false } } };
const json = (body: unknown, status = 200) => ({ ok: status < 300, status, json: async () => body });
let fail: string | null;
let outcome: string | number;
let requests: Array<{ url: string; options: RequestInit }>;
let delay: (() => Promise<unknown>) | null;
const failed = async () => {
  if (delay) return delay();
  if (outcome === "transport") throw new Error("owned transport refusal");
  if (outcome === "non-json") return { ok: false, status: 500, json: async () => { throw new Error("not JSON"); } };
  return json({ detail: "owned refusal" }, outcome as number);
};
beforeEach(() => {
  const store = new Map<string,string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value), removeItem: (key: string) => store.delete(key), clear: () => store.clear() });
  localStorage.setItem("vectoraiz_api_url", "https://owned.invalid");
  localStorage.setItem("aim_data_access_token", "owned-fixture-token");
  requests = []; fail = null; outcome = 500; delay = null; notice.mockReset();
  // All transport is owned; an unexpected URL fails instead of reaching network.
  vi.stubGlobal("fetch", vi.fn(async (url: string, options: RequestInit = {}) => {
    requests.push({ url, options });
    if (fail && url.includes(fail)) return failed();
    if (url.endsWith("/api/settings/portal") && options.method === "PUT") return json(config);
    if (url.endsWith("/api/settings/portal")) return json(config);
    if (url.endsWith("/api/datasets")) return json({ datasets: [{ id: "owned", original_filename: "Owned dataset" }] });
    if (url.endsWith("/datasets/owned")) return json({ portal_visible: true });
    if (url.endsWith("/revoke-sessions")) return json({ message: "confirmed" });
    if (url.endsWith("/test-oidc")) return json({ success: true });
    if (url.includes("/access-logs?")) return json({ logs: [] });
    throw new Error("Unexpected request: " + url);
  }));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
async function loaded() { render(<PortalSettingsSection />); await screen.findByText("7 active sessions"); }
function visibility() { return screen.queryByRole("switch", { name: "Visibility for Owned dataset" }) ?? screen.getAllByRole("switch")[1]; }

describe("R305 real portal section through authenticated apiFetch", () => {
  for (const path of ["/api/settings/portal", "/api/datasets"]) {
    it(`read refusal at ${path} is unavailable, blocks writes and retries`, async () => {
      fail = path;
      render(<PortalSettingsSection />);
      await screen.findByText(/Portal settings unavailable/);
      expect(screen.queryByText("Save Portal Settings")).toBeNull();
      expect(screen.queryByText("7 active sessions")).toBeNull();
      fail = null;
      fireEvent.click(screen.getByText("Retry portal settings"));
      await screen.findByText("7 active sessions");
      expect(visibility()).toHaveAttribute("aria-checked", "false");
    });
  }
  for (const failure of [401, 403, 404, 500, "non-json", "transport"]) {
    it(`revoke ${failure} retains sessions and reports failure`, async () => {
      await loaded(); fail = "/revoke-sessions"; outcome = failure;
      fireEvent.click(screen.getByText("Revoke All"));
      await waitFor(() => expect(notice).toHaveBeenCalledWith(expect.objectContaining({ title: "Failed to revoke sessions", variant: "destructive" })));
      expect(screen.getByText("7 active sessions")).toBeInTheDocument();
      expect(notice).not.toHaveBeenCalledWith({ title: "All portal sessions revoked" });
    });
    it(`visibility ${failure} retains confirmed setting`, async () => {
      await loaded(); fail = "/datasets/owned"; outcome = failure;
      fireEvent.click(visibility());
      await waitFor(() => expect(notice).toHaveBeenCalledWith(expect.objectContaining({ title: "Failed to update dataset visibility" })));
      expect(visibility()).toHaveAttribute("aria-checked", "false");
    });
  }
  it("successful controls use Bearer and cookie transport consistently", async () => {
    await loaded();
    fireEvent.click(visibility());
    await waitFor(() => expect(visibility()).toHaveAttribute("aria-checked", "true"));
    fireEvent.click(screen.getByText("Save Portal Settings"));
    await waitFor(() => expect(notice).toHaveBeenCalledWith({ title: "Portal settings saved" }));
    fireEvent.click(screen.getByText("Test Connection"));
    await screen.findByText("Connection successful");
    fireEvent.click(screen.getByText("View SSO Access Logs"));
    await screen.findByText("No access logs yet.");
    fireEvent.click(screen.getByText("Revoke All"));
    await waitFor(() => expect(notice).toHaveBeenCalledWith({ title: "All portal sessions revoked" }));
    expect(screen.queryByText("7 active sessions")).toBeNull();
    expect(requests.length).toBe(7);
    for (const { options } of requests) {
      expect(options.credentials).toBe("include");
      expect(options.headers).toEqual(expect.objectContaining({ Authorization: "Bearer owned-fixture-token" }));
    }
  });
  it("failed logs and OIDC never display empty logs or success; logs can retry", async () => {
    await loaded(); fail = "/access-logs?";
    fireEvent.click(screen.getByText("View SSO Access Logs"));
    await screen.findByText(/Access logs unavailable/);
    expect(screen.queryByText("No access logs yet.")).toBeNull();
    fireEvent.click(screen.getByText("Hide Access Logs")); fail = null;
    fireEvent.click(screen.getByText("View SSO Access Logs"));
    await screen.findByText("No access logs yet."); fail = "/test-oidc";
    fireEvent.click(screen.getByText("Test Connection"));
    await screen.findByText("Error: Connection failed");
    expect(screen.queryByText("Connection successful")).toBeNull();
  });
  it("pending visibility serializes controls and commits only returned state", async () => {
    await loaded(); fail = "/datasets/owned";
    let resolve!: (value: unknown) => void;
    delay = () => new Promise(r => { resolve = r; });
    fireEvent.click(visibility());
    expect(visibility()).toBeDisabled();
    expect(screen.getByText("Save Portal Settings")).toBeDisabled();
    fireEvent.click(visibility());
    fireEvent.click(screen.getByText("Revoke All"));
    expect(requests.filter(r => r.url.endsWith("/datasets/owned"))).toHaveLength(1);
    expect(requests.some(r => r.url.endsWith("/revoke-sessions"))).toBe(false);
    await act(async () => resolve(json({ portal_visible: false })));
    await waitFor(() => expect(visibility()).not.toBeDisabled());
    expect(visibility()).toHaveAttribute("aria-checked", "false");
  });
  it("save refusal preserves confirmed sessions and reports error", async () => {
    await loaded(); fail = "/api/settings/portal";
    fireEvent.click(screen.getByText("Save Portal Settings"));
    await waitFor(() => expect(notice).toHaveBeenCalledWith(expect.objectContaining({ variant: "destructive" })));
    expect(screen.getByText("7 active sessions")).toBeInTheDocument();
    expect(notice).not.toHaveBeenCalledWith({ title: "Portal settings saved" });
  });
  it("reuses the existing refresh flow and preserves cookie options on retry", async () => {
    localStorage.setItem("aim_data_refresh_token", "owned-refresh-fixture");
    const transport = fetch;
    let refused = false;
    vi.stubGlobal("fetch", vi.fn(async (url: string, options: RequestInit = {}) => {
      if (url.endsWith("/api/auth/aim-market-refresh")) return json({ access_token: "owned-refreshed-fixture", refresh_token: "owned-next-refresh" });
      if (!refused) { refused = true; return json({ detail: "expired fixture" }, 401); }
      return transport(url, options);
    }));
    await loaded();
    expect(localStorage.getItem("aim_data_access_token")).toBe("owned-refreshed-fixture");
    expect(requests.every(r => (r.options.headers as Record<string,string>).Authorization === "Bearer owned-refreshed-fixture")).toBe(true);
    expect(requests.every(r => r.options.credentials === "include")).toBe(true);
  });
  it("cookie requests without a stored Bearer still use credentials include", async () => {
    localStorage.removeItem("aim_data_access_token");
    await loaded();
    expect(requests.every(r => r.options.credentials === "include")).toBe(true);
    expect(requests.every(r => !(r.options.headers as Record<string,string> | undefined)?.Authorization)).toBe(true);
  });
});
