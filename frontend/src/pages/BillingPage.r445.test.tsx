import type { ButtonHTMLAttributes, HTMLAttributes, InputHTMLAttributes, LabelHTMLAttributes } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import BillingPage from "./BillingPage";

const state = vi.hoisted(() => ({ params: new URLSearchParams(), setParams: vi.fn(), toast: vi.fn(), toggle: undefined as undefined | ((checked: boolean) => void) }));
vi.mock("react-router-dom", () => ({ useSearchParams: () => [state.params, state.setParams] }));
vi.mock("@/contexts/BrandContext", () => ({ useBrand: () => ({ name: "Synthetic brand" }) }));
vi.mock("@/lib/api", () => ({ getApiUrl: () => "http://synthetic.invalid" }));
vi.mock("@/hooks/use-toast", () => ({ toast: state.toast }));
vi.mock("@/components/ui/card", () => {
  const Card = (props: HTMLAttributes<HTMLDivElement>) => <div {...props} />;
  return { Card, CardContent: Card };
});
vi.mock("@/components/ui/button", () => ({ Button: ({ variant: _variant, size: _size, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: string; size?: string }) => <button {...props} /> }));
vi.mock("@/components/ui/input", () => ({ Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} /> }));
vi.mock("@/components/ui/label", () => ({ Label: (props: LabelHTMLAttributes<HTMLLabelElement>) => <label {...props} /> }));
vi.mock("@/components/ui/progress", () => ({ Progress: () => null }));
vi.mock("@/components/ui/switch", () => ({ Switch: ({ checked, onCheckedChange, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { checked: boolean; onCheckedChange: (checked: boolean) => void }) => {
  state.toggle = onCheckedChange;
  return <button {...props} role="switch" aria-checked={checked} onClick={() => onCheckedChange(!checked)} />;
} }));

type Config = { enabled: boolean; threshold_usd: number; reload_amount_usd: number };
const config = (enabled = false, threshold_usd = 5): Config => ({ enabled, threshold_usd, reload_amount_usd: 25 });
const response = (data: unknown, ok = true) => ({ ok, status: ok ? 200 : 400, json: async () => data }) as Response;
function deferred() {
  let resolve!: (value: Response) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<Response>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
let saved: Config;
let read: () => Promise<Response>;
let save: (init: RequestInit) => Promise<Response>;
let calls: Array<{ url: string; init: RequestInit }>;
const mutations = () => calls.filter(({ init }) => init.method);
const reads = () => calls.filter(({ url, init }) => url.endsWith("/auto-reload") && !init.method);
const toggle = () => screen.getByRole("switch") as HTMLButtonElement;

beforeEach(() => {
  saved = config(); calls = []; state.toast.mockClear(); state.params = new URLSearchParams(); state.toggle = undefined;
  read = async () => response(saved);
  save = async (init) => response(JSON.parse(String(init.body)));
  vi.stubGlobal("localStorage", { getItem: (key: string) => key === "aim_data_access_token" ? "synthetic-token" : null });
  vi.stubGlobal("XMLHttpRequest", vi.fn(() => { throw new Error("XHR denied"); }));
  vi.stubGlobal("crypto", { randomUUID: () => "00000000-0000-4000-8000-000000000001" });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    calls.push({ url, init });
    if (url === "http://synthetic.invalid/api/allai/credits/auto-reload") {
      if (!init.method) return read();
      if (init.method === "POST") return save(init);
    }
    if (init.method) throw new Error("Unexpected mocked mutation");
    if (url === "http://synthetic.invalid/api/allai/credits") return response({ balance_usd: 10, credit_usd: 25, used_usd: 15, usage: [] });
    if (url === "http://synthetic.invalid/api/allai/credits/auto-reload/pending") return response({ pending: false });
    if (url === "http://synthetic.invalid/api/allai/account") return response({ has_account: true });
    throw new Error("Unexpected mocked request");
  }));
});
afterEach(() => {
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  expect(calls.every(({ url }) => url.startsWith("http://synthetic.invalid/api/allai/"))).toBe(true);
  cleanup(); vi.unstubAllGlobals();
});
async function mount() {
  render(<BillingPage />);
  await screen.findByText("$10.00");
  await waitFor(() => expect(toggle().getAttribute("aria-checked")).toBe(String(saved.enabled)));
}
async function settledSave() { await waitFor(() => expect(state.toast).toHaveBeenCalled()); }

it("retains confirmed off after HTTP-rejected enable", async () => {
  save = async () => response({ detail: "Synthetic rejection" }, false);
  await mount(); fireEvent.click(toggle()); await settledSave();
  expect(toggle()).toHaveAttribute("aria-checked", "false");
  expect(state.toast).toHaveBeenCalledWith({ title: "Error", description: "Synthetic rejection", variant: "destructive" });
});
it("retains confirmed on after transport-failed disable", async () => {
  saved = config(true); save = async () => { throw new Error("Synthetic network failure"); };
  await mount(); fireEvent.click(toggle()); await settledSave();
  expect(toggle()).toHaveAttribute("aria-checked", "true");
  expect(screen.getByText("Reload to $25 when balance reaches $5")).toBeTruthy();
});
it("keeps confirmed state pending and rejects repeated toggle callbacks before rerender", async () => {
  const pending = deferred(); save = () => pending.promise;
  await mount();
  const callback = state.toggle!;
  act(() => { callback(true); callback(true); callback(false); });
  expect(toggle()).toHaveAttribute("aria-checked", "false"); expect(toggle()).toBeDisabled();
  fireEvent.click(toggle()); expect(mutations()).toHaveLength(1);
  await act(async () => pending.resolve(response(config(true))));
  expect(toggle()).toHaveAttribute("aria-checked", "true"); expect(toggle()).not.toBeDisabled();
});
it.each([false, true])("renders ordinary confirmed %s without mutation", async (enabled) => {
  saved = config(enabled, 9); await mount();
  expect(toggle()).toHaveAttribute("aria-checked", String(enabled)); expect(mutations()).toHaveLength(0);
  if (enabled) expect(screen.getByText("Reload to $25 when balance reaches $9")).toBeTruthy();
});
it("preserves ordinary successful enable and its exact request body", async () => {
  await mount(); fireEvent.click(toggle()); await settledSave();
  expect(toggle()).toHaveAttribute("aria-checked", "true");
  expect(state.toast).toHaveBeenCalledWith({ title: "Auto-reload saved" });
  expect(mutations()).toHaveLength(1);
  expect(JSON.parse(String(mutations()[0].init.body))).toEqual(config(true));
});
it("adopts successful normalized enable response and preserves exact POST contract", async () => {
  save = async () => response(config(true, 7)); await mount(); fireEvent.click(toggle()); await settledSave();
  expect(toggle()).toHaveAttribute("aria-checked", "true");
  expect(screen.getByText("Reload to $25 when balance reaches $7")).toBeTruthy();
  expect(mutations()).toEqual([{ url: "http://synthetic.invalid/api/allai/credits/auto-reload", init: {
    method: "POST", headers: { Authorization: "Bearer synthetic-token", "Content-Type": "application/json" }, body: JSON.stringify(config(true)),
  } }]);
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByLabelText("Reload when balance drops below")).toHaveValue(7);
});
it("adopts successful disable", async () => {
  saved = config(true); await mount(); fireEvent.click(toggle()); await settledSave();
  expect(toggle()).toHaveAttribute("aria-checked", "false"); expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
});
it("offers no fabricated setting during pending initial read", async () => {
  const pending = deferred(); read = () => pending.promise;
  render(<BillingPage />); await screen.findByText("$10.00");
  const control = screen.queryByRole("switch"); expect(!control || (control as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("Loading auto-reload settings...")).toBeTruthy(); expect(mutations()).toHaveLength(0);
  await act(async () => pending.resolve(response(config(true, 8))));
  expect(toggle()).toHaveAttribute("aria-checked", "true");
});
it.each(["http", "network"])("offers no fabricated setting after %s initial read failure", async (failure) => {
  read = async () => { if (failure === "network") throw new Error("Synthetic read failure"); return response({}, false); };
  render(<BillingPage />); await screen.findByText("$10.00"); await act(async () => {});
  const control = screen.queryByRole("switch"); expect(!control || (control as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText("Auto-reload settings unavailable.")).toBeTruthy(); expect(mutations()).toHaveLength(0);
});
it("retries reads only on request and rejects overlapping retry so no stale completion can win", async () => {
  read = async () => response({}, false); render(<BillingPage />);
  const retry = await screen.findByRole("button", { name: "Retry auto-reload settings" });
  expect(reads()).toHaveLength(1);
  const pending = deferred(); read = () => pending.promise;
  fireEvent.click(retry); fireEvent.click(retry); expect(retry).toBeDisabled(); expect(reads()).toHaveLength(2);
  await act(async () => pending.resolve(response(config(true, 11))));
  expect(screen.queryByRole("button", { name: "Retry auto-reload settings" })).toBeNull();
  expect(screen.getByText("Reload to $25 when balance reaches $11")).toBeTruthy();
  expect(reads()[1].init).toEqual({ headers: { Authorization: "Bearer synthetic-token" } });
});
it("preserves confirmed threshold and locks toggle, edit, draft and cancel during threshold save", async () => {
  saved = config(true); const pending = deferred(); save = () => pending.promise;
  await mount(); fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  const input = screen.getByLabelText("Reload when balance drops below"); fireEvent.change(input, { target: { value: "12" } });
  const saveButton = screen.getByRole("button", { name: "Save" }); fireEvent.click(saveButton); fireEvent.click(saveButton);
  expect(input).toBeDisabled(); expect(toggle()).toBeDisabled(); expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  fireEvent.click(toggle()); act(() => state.toggle!(false)); expect(mutations()).toHaveLength(1);
  expect(screen.getByText("Reload to $25 when balance reaches $5")).toBeTruthy();
  await act(async () => pending.resolve(response(config(true, 10))));
  expect(screen.getByText("Reload to $25 when balance reaches $10")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Edit" })); expect(screen.getByLabelText("Reload when balance drops below")).toHaveValue(10);
  expect(JSON.parse(String(mutations()[0].init.body))).toEqual(config(true, 12));
});
it("blocks edit while a disable save is pending", async () => {
  saved = config(true); const pending = deferred(); save = () => pending.promise; await mount();
  const edit = screen.getByRole("button", { name: "Edit" }); fireEvent.click(toggle()); fireEvent.click(edit);
  expect(edit).toBeDisabled(); expect(screen.queryByLabelText("Reload when balance drops below")).toBeNull();
  await act(async () => pending.reject(new Error("Synthetic failed disable")));
  expect(toggle()).toHaveAttribute("aria-checked", "true"); expect(edit).not.toBeDisabled();
});
it("keeps rejected threshold draft available for successful retry", async () => {
  saved = config(true); save = async () => response({ detail: "Synthetic threshold rejection" }, false); await mount();
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  const input = screen.getByLabelText("Reload when balance drops below"); fireEvent.change(input, { target: { value: "13" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" })); await settledSave();
  expect(input).toHaveValue(13); expect(input).not.toBeDisabled(); expect(screen.getByText("Reload to $25 when balance reaches $5")).toBeTruthy();
  state.toast.mockClear(); save = async () => response(config(true, 13)); fireEvent.click(screen.getByRole("button", { name: "Save" })); await settledSave();
  expect(screen.queryByLabelText("Reload when balance drops below")).toBeNull(); expect(screen.getByText("Reload to $25 when balance reaches $13")).toBeTruthy();
  expect(mutations().map(({ init }) => JSON.parse(String(init.body)))).toEqual([config(true, 13), config(true, 13)]);
});
it("preserves threshold validation, fixed amount and cancel draft reset", async () => {
  saved = config(true, 6); await mount(); fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByText("$25.00 (fixed)")).toBeTruthy();
  for (const value of ["", "0"]) {
    fireEvent.change(screen.getByLabelText("Reload when balance drops below"), { target: { value } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(state.toast).toHaveBeenLastCalledWith({ title: "Invalid threshold", description: "Must be at least $1", variant: "destructive" });
  }
  expect(mutations()).toHaveLength(0); fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  fireEvent.click(screen.getByRole("button", { name: "Edit" })); expect(screen.getByLabelText("Reload when balance drops below")).toHaveValue(6);
});
