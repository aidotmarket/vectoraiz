import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { PortalChatEvent } from "@/api/portalApi";

// Install tripwires before application imports. Only chatStream is doubled below.
const network = await vi.hoisted(async () => {
  const blocked = vi.fn(() => { throw new Error("Unexpected network access in portal UI test"); });
  const http = (await import("node:http")).default;
  const https = (await import("node:https")).default;
  const net = (await import("node:net")).default;
  const tls = (await import("node:tls")).default;
  vi.stubGlobal("fetch", blocked);
  vi.stubGlobal("WebSocket", blocked);
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(blocked);
  vi.spyOn(http, "request").mockImplementation(blocked);
  vi.spyOn(http, "get").mockImplementation(blocked);
  vi.spyOn(https, "request").mockImplementation(blocked);
  vi.spyOn(https, "get").mockImplementation(blocked);
  vi.spyOn(net.Socket.prototype, "connect").mockImplementation(blocked);
  vi.spyOn(tls, "connect").mockImplementation(blocked);
  return blocked;
});

import { portalApi } from "@/api/portalApi";
import PortalAllAIChat from "./PortalAllAIChat";

const disabled = "Allie requires an ai.market connection. Switch to connected mode to use Allie.";
type Messages = Parameters<typeof portalApi.chatStream>[0];
let requests: Messages[];

beforeEach(() => {
  requests = [];
  network.mockClear();
  // jsdom has no layout/scrolling implementation.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  cleanup();
  expect(network).not.toHaveBeenCalled();
  vi.mocked(portalApi.chatStream).mockRestore();
});

afterAll(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function stream(events: PortalChatEvent[], thrown?: Error) {
  vi.spyOn(portalApi, "chatStream").mockImplementation(async function* (messages) {
    requests.push(messages.map((message) => ({ ...message })));
    // Synthetic equivalent of content min_length=1: invalid history cannot
    // reach the disabled SSE response on the next submission.
    if (messages.some((message) => message.content.length === 0)) {
      throw Object.assign(new Error("Invalid message content (422)"), { status: 422 });
    }
    for (const event of events) yield event;
    if (thrown) throw thrown;
  });
}

function openChat() {
  const view = render(<PortalAllAIChat />);
  fireEvent.click(screen.getByRole("button", { name: "Open AI chat" }));
  return view;
}

async function send(text: string) {
  const input = screen.getByPlaceholderText("Ask about your data...");
  fireEvent.change(input, { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() => expect(input).toBeEnabled());
  expect(screen.getByText(text)).toBeVisible();
}

describe("portal chat response lifecycle", () => {
  it("allows two consecutive disabled SSE responses with nonempty history and no loading bubble", async () => {
    stream([{ type: "error", text: disabled }]);
    const { container } = openChat();
    await send("First question");
    expect(screen.getByText(disabled)).toBeVisible();
    const firstLoadingBubbles = container.querySelectorAll(".animate-spin").length;
    await send("Second question");
    expect.soft(requests).toEqual([
      [{ role: "user", content: "First question" }],
      [{ role: "user", content: "First question" }, { role: "user", content: "Second question" }],
    ]);
    expect.soft(firstLoadingBubbles).toBe(0);
    expect.soft(container.querySelector(".animate-spin")).toBeNull();
    expect.soft(screen.queryAllByText(disabled)).toHaveLength(1);
    expect.soft(screen.queryByText("Invalid message content (422)")).not.toBeInTheDocument();
  });

  it("preserves partial assistant text before a streamed error in the UI and next request", async () => {
    stream([{ type: "chunk", text: "Partial " }, { type: "chunk", text: "answer" }, { type: "error", text: "Stream interrupted" }]);
    const { container } = openChat();
    await send("First question");
    expect(screen.getByText("Partial answer")).toBeVisible();
    expect(screen.getByText("Stream interrupted")).toBeVisible();
    await send("Second question");
    expect(requests[1]).toEqual([
      { role: "user", content: "First question" },
      { role: "assistant", content: "Partial answer" },
      { role: "user", content: "Second question" },
    ]);
    expect(screen.getAllByText("Partial answer")).toHaveLength(2);
    expect(container.querySelector(".animate-spin")).toBeNull();
  });

  it("renders normal chunks and retains the completed answer on the next send", async () => {
    stream([{ type: "chunk", text: "Full " }, { type: "chunk", text: "answer" }, { type: "done", text: "" }]);
    const { container } = openChat();
    await send("First question");
    expect(screen.getByText("Full answer")).toBeVisible();
    await send("Second question");
    expect(requests[1]).toEqual([
      { role: "user", content: "First question" },
      { role: "assistant", content: "Full answer" },
      { role: "user", content: "Second question" },
    ]);
    expect(container.querySelector(".animate-spin")).toBeNull();
    expect(container.querySelector(".text-destructive")).toBeNull();
  });

  it.each([
    [Object.assign(new Error("Server rate limit detail"), { status: 429 }), "Rate limit reached. Please try again later."],
    [new Error("Transport disconnected"), "Transport disconnected"],
    [new Error(""), "Failed to send message."],
  ])("keeps thrown error copy and removes empty placeholders (%s)", async (thrown, copy) => {
    stream([], thrown);
    const { container } = openChat();
    await send("First question");
    expect(screen.getByText(copy)).toBeVisible();
    expect(container.querySelector(".animate-spin")).toBeNull();
    await send("Second question");
    expect(requests[1]).toEqual([
      { role: "user", content: "First question" },
      { role: "user", content: "Second question" },
    ]);
    expect(screen.getAllByText(copy)).toHaveLength(1);
  });

  it("preserves partial text when transport throws", async () => {
    stream([{ type: "chunk", text: "Partial answer" }], new Error("Transport disconnected"));
    const { container } = openChat();
    await send("First question");
    expect(screen.getByText("Partial answer")).toBeVisible();
    expect(screen.getByText("Transport disconnected")).toBeVisible();
    expect(container.querySelector(".animate-spin")).toBeNull();
  });
});
