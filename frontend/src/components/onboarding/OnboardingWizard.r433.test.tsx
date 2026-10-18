import { Component, type ReactNode } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OnboardingWizard, { isOnboardingComplete } from "./OnboardingWizard";
import { BrandProvider } from "@/contexts/BrandContext";

// Exercise the actual wizard and brand context without app/bootstrap imports.
// Native presentation keeps these tests isolated from Radix portals and focus.
vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: ReactNode }) => <>{children}</>,
  DialogContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick }: { children: ReactNode; onClick: () => void }) =>
    <button onClick={onClick}>{children}</button>,
}));

class Boundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    return this.state.failed ? <div role="alert">Tour crashed</div> : this.props.children;
  }
}

const click = (name: string) => fireEvent.click(screen.getByRole("button", { name }));
const tour = (channel: "direct" | "marketplace" | "aim-data", onComplete: () => void) => (
  <Boundary><BrandProvider><OnboardingWizard channel={channel} onComplete={onComplete} /></BrandProvider></Boundary>
);

beforeEach(() => {
  localStorage.clear();
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.stubGlobal("XMLHttpRequest", vi.fn(() => { throw new Error("Unexpected network"); }));
});
afterEach(() => {
  cleanup();
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("channel-owned onboarding progress", () => {
  it("safely restarts a late direct fourth step as aim-data step one without completing", () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const done = vi.fn();
    const view = render(tour("direct", done));
    for (let i = 0; i < 3; i++) click("Next");
    expect(screen.getByRole("heading", { name: "Ready!" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Get Started" })).toBeTruthy();

    view.rerender(tour("aim-data", done));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(errors).not.toHaveBeenCalled();
    expect(screen.getByText("Turn files into sellable data products")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Get Started" })).toBeNull();
    expect(done).not.toHaveBeenCalled();
    expect(isOnboardingComplete()).toBe(false);
    click("Next");
    expect(screen.getByRole("heading", { name: "Upload your first file" })).toBeTruthy();
    click("Next");
    expect(screen.getByRole("heading", { name: "Publish to ai.market" })).toBeTruthy();
    click("Get Started");
    expect(done).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem("vz-onboarding-complete")).toBe("true");
  });

  it("retains progress and uses the latest callback on a same-channel rerender", () => {
    const oldDone = vi.fn();
    const newDone = vi.fn();
    const view = render(tour("direct", oldDone));
    click("Next");
    view.rerender(tour("direct", newDone));
    expect(screen.getByRole("heading", { name: "Vectorize & Index" })).toBeTruthy();
    click("Next");
    click("Next");
    view.rerender(tour("direct", newDone));
    expect(screen.getByRole("heading", { name: "Ready!" })).toBeTruthy();
    click("Get Started");
    expect(oldDone).not.toHaveBeenCalled();
    expect(newDone).toHaveBeenCalledTimes(1);
  });

  it("restarts on equal-length channel changes and when returning to a previous channel", () => {
    const done = vi.fn();
    const view = render(tour("direct", done));
    click("Next");
    view.rerender(tour("marketplace", done));
    expect(screen.getByText("Connect to your ai.market account")).toBeTruthy();
    click("Next");
    view.rerender(tour("direct", done));
    expect(screen.getByText("Upload your data")).toBeTruthy();
    expect(done).not.toHaveBeenCalled();
    expect(isOnboardingComplete()).toBe(false);
  });

  it.each([
    ["direct", ["Upload your data", "Choose embedding model", "Try a RAG query", "You\u0027re all set"]],
    ["marketplace", ["Connect to your ai.market account", "Add your dataset", "Review metadata and set pricing", "List on ai.market"]],
    ["aim-data", ["Turn files into sellable data products", "Bring in any format", "Create your listing"]],
  ] as const)("completes the ordinary %s tour in order and persists completion", (channel, subtitles) => {
    const done = vi.fn();
    render(tour(channel, done));
    subtitles.forEach((subtitle, index) => {
      expect(screen.getByText(subtitle)).toBeTruthy();
      expect(done).not.toHaveBeenCalled();
      expect(isOnboardingComplete()).toBe(false);
      click(index === subtitles.length - 1 ? "Get Started" : "Next");
    });
    expect(done).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem("vz-onboarding-complete")).toBe("true");
    expect(isOnboardingComplete()).toBe(true);
  });

  it("can Skip when completion storage is unavailable", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("Storage disabled"); });
    const write = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Storage disabled"); });
    const done = vi.fn();
    render(tour("aim-data", done));
    expect(isOnboardingComplete()).toBe(false);
    click("Skip");
    expect(write).toHaveBeenCalledExactlyOnceWith("vz-onboarding-complete", "true");
    expect(done).toHaveBeenCalledTimes(1);
  });
});
