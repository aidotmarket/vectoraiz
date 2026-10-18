import { cleanup, render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import AiMarketPage from "./AiMarketPage";

const state = vi.hoisted(() => ({
  mode: { isStandalone: false, isLoading: false, readStatus: "confirmed" as "pending" | "confirmed" | "failed", retryConfiguration: async () => {}, hasFeature: () => true },
  assistant: { isStandalone: false, connectionStatus: "connected", allieAvailable: true },
  network: vi.fn(() => { throw new Error("NETWORK DENIED"); }),
}));

vi.mock("@/contexts/ModeContext", () => ({ useMode: () => state.mode }));
vi.mock("@/contexts/CoPilotContext", () => ({ useCoPilot: () => state.assistant }));

beforeEach(() => {
  state.mode = { isStandalone: false, isLoading: false, readStatus: "confirmed" as "pending" | "confirmed" | "failed", retryConfiguration: async () => {}, hasFeature: () => true };
  state.assistant = { isStandalone: false, connectionStatus: "connected", allieAvailable: true };
  state.network.mockClear();
  vi.stubGlobal("fetch", state.network);
  vi.stubGlobal("XMLHttpRequest", state.network);
  vi.stubGlobal("WebSocket", state.network);
  vi.stubGlobal("crypto", { randomUUID: () => "owned-status-fixture" });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  expect(state.network).not.toHaveBeenCalled();
});

const page = () => <MemoryRouter><AiMarketPage /></MemoryRouter>;
const setup = "./start.sh --setup-allie";
const active = "allAI is active";

// Exercise independent inputs, including stale availability after a socket closes.
for (const isLoading of [true, false]) {
  for (const isStandalone of [true, false]) {
    for (const allieStandalone of [true, false]) {
      for (const connectionStatus of ["connected", "connecting", "disconnected"]) {
        for (const allieAvailable of [true, false]) {
          it(`loading=${isLoading}, modeStandalone=${isStandalone}, assistantStandalone=${allieStandalone}, socket=${connectionStatus}, available=${allieAvailable}`, () => {
            Object.assign(state.mode, { isLoading, isStandalone });
            Object.assign(state.assistant, { isStandalone: allieStandalone, connectionStatus, allieAvailable });
            const view = render(page());
            const text = view.container.textContent || "";
            const agrees = isStandalone === allieStandalone;
            const expectedActive = !isLoading && agrees && !isStandalone
              && connectionStatus === "connected" && allieAvailable;
            expect(text.includes(active)).toBe(expectedActive);
            expect(text.includes(setup)).toBe(!isLoading && agrees && isStandalone);
            expect(text).not.toContain("Connected to ai.market");
            expect(text).not.toContain("API Key");
            expect(text).not.toContain("am_****");
            if (isLoading) {
              expect(text).toContain("Checking ai.market configuration");
              expect(text).toContain("Checking allAI availability");
            } else if (!agrees) {
              expect(text).toContain("Connection status is unknown");
              expect(text).toContain("allAI availability is unknown while configuration updates");
            } else if (isStandalone) {
              expect(text).toContain("Not connected");
            } else {
              expect(text).toContain("Configured for ai.market");
              expect(text).toContain("remote ai.market connection has not been verified here");
              expect(text).toContain("Local assistant connection");
              expect(text).toContain(connectionStatus);
              if (connectionStatus === "connecting") expect(text).toContain("Connecting to the local assistant");
              if (connectionStatus === "disconnected") expect(text).toContain("Local assistant disconnected");
              if (connectionStatus === "connected" && !allieAvailable) expect(text).toContain("allAI is currently unavailable");
            }
          });
        }
      }
    }
  }
}

it("updates mounted guidance through loading, configuration disagreement, socket loss and recovery", () => {
  Object.assign(state.mode, { isLoading: true, isStandalone: true });
  Object.assign(state.assistant, { isStandalone: true, allieAvailable: false, connectionStatus: "disconnected" });
  const view = render(page());
  expect(view.container.textContent).not.toContain(setup);
  state.mode.isLoading = false;
  view.rerender(page());
  expect(view.container.textContent).toContain(setup);
  state.mode.isStandalone = false;
  view.rerender(page());
  expect(view.container.textContent).toContain("Connection status is unknown");
  expect(view.container.textContent).not.toContain(setup);
  Object.assign(state.assistant, { isStandalone: false, allieAvailable: true, connectionStatus: "connected" });
  view.rerender(page());
  expect(view.container.textContent).toContain(active);
  state.assistant.connectionStatus = "disconnected";
  view.rerender(page());
  expect(view.container.textContent).not.toContain(active);
  expect(view.container.textContent).not.toContain(setup);
  state.assistant.connectionStatus = "connecting";
  view.rerender(page());
  expect(view.container.textContent).toContain("Connecting to the local assistant");
  state.assistant.connectionStatus = "connected";
  state.assistant.allieAvailable = false;
  view.rerender(page());
  expect(view.container.textContent).toContain("allAI is currently unavailable");
  state.assistant.allieAvailable = true;
  view.rerender(page());
  expect(view.container.textContent).toContain(active);
  state.mode.isLoading = true;
  view.rerender(page());
  expect(view.container.textContent).not.toContain(active);
});

it("preserves the marketplace feature gate and earnings destination", () => {
  const view = render(page());
  expect(view.getByRole("link", { name: /View Earnings Dashboard/ }).getAttribute("href")).toBe("/earnings");
  state.mode.hasFeature = () => false;
  view.rerender(page());
  expect(view.queryByRole("link", { name: /View Earnings Dashboard/ })).toBeNull();
  expect(view.container.textContent).toContain("Coming Soon");
  state.mode.hasFeature = () => true;
  state.mode.isStandalone = true;
  state.assistant.isStandalone = true;
  view.rerender(page());
  expect(view.queryByRole("link", { name: /View Earnings Dashboard/ })).toBeNull();
  expect(view.container.textContent).toContain("Coming Soon");
});
