import React from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Dataset } from "@/types/mockDatasets";

const h = vi.hoisted(() => ({
  deny: vi.fn(() => { throw new Error("Unexpected transport, write or publish action"); }),
  scan: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ piiApi: { getScan: h.scan }, getApiUrl: h.deny }));
vi.mock("@/contexts/AuthContext", () => ({ useAuth: () => ({ apiKey: null }) }));
vi.mock("@/hooks/use-toast", () => ({ toast: h.deny }));
import PublishModal from "./PublishModal";

const dataset: Dataset = {
  id: "synthetic-r468-weather", name: "rainfall.csv", rows: 12, columns: 2,
  type: "csv", status: "ready", size: "1 KB", sizeBytes: 1024, processingTime: 0,
  createdAt: new Date("2026-10-01"), modifiedAt: new Date("2026-10-01"),
};

beforeEach(() => {
  h.scan.mockResolvedValue({ column_results: [] });
  for (const name of ["fetch", "XMLHttpRequest", "WebSocket"] as const) vi.stubGlobal(name, h.deny);
  vi.spyOn(window, "open").mockImplementation(h.deny);
  for (const method of ["setItem", "removeItem", "clear"] as const) {
    vi.spyOn(Storage.prototype, method).mockImplementation(h.deny);
  }
});
afterEach(() => {
  try {
    cleanup();
    expect(h.deny).not.toHaveBeenCalled();
  } finally {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  }
});

async function openPricing() {
  render(<PublishModal open onOpenChange={h.deny} dataset={dataset} onPublishSuccess={h.deny} />);
  await act(async () => {});
  fireEvent.click(screen.getByLabelText("I confirm I have reviewed the privacy implications"));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  fireEvent.change(screen.getByLabelText("Description"), {
    target: { value: "Twelve synthetic rainfall observations; research use rights only." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("heading", { name: "Set Your Price" })).toBeVisible();
  expect(h.scan).toHaveBeenCalledExactlyOnceWith(dataset.id);
}

function expectSplit(seller: string, fee: string) {
  const feeCard = screen.getByText("Platform Fee (5%)").parentElement!;
  const sellerCard = screen.getByText("Your share before Stripe fees (95%)").parentElement!;
  expect(within(feeCard).getByText(fee, { exact: true })).toBeVisible();
  expect(within(sellerCard).getByText(seller, { exact: true })).toBeVisible();
  expect(screen.getByText("Actual Stripe fees are deducted from this amount.")).toBeVisible();
}

describe("R468 truthful publish price guidance", () => {
  it("does not present a fixed range as comparable dataset prices", async () => {
    await openPricing();
    expect(screen.queryByText(/Similar datasets:/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/\$200\s*-\s*\$800/)).not.toBeInTheDocument();
    expect(screen.queryByText("Pricing Suggestions")).not.toBeInTheDocument();
    expect(screen.getByText("Market price comparisons and recommendations are unavailable here.")).toBeVisible();
  });

  it("does not present a fixed recommended price for seller metadata", async () => {
    await openPricing();
    expect(screen.queryByText(/Recommended:/i)).not.toBeInTheDocument();
    expect(screen.queryByText("$450", { exact: true })).not.toBeInTheDocument();
    expect(screen.getByText("Choose your price based on the actual dataset and the use rights you offer.")).toBeVisible();
  });

  it("preserves the manual USD price, fee rounding and navigation without publishing", async () => {
    await openPricing();
    const price = screen.getByRole("spinbutton", { name: "Price (USD)" });
    expect(price).toHaveValue(450);
    expect(price).toHaveAttribute("min", "0");
    expect(price).toHaveAttribute("step", "10");
    expectSplit("$427.50", "$22.50");
    fireEvent.change(price, { target: { value: "12.34" } });
    expectSplit("$11.73", "$0.61");
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByLabelText("Description")).toHaveValue(
      "Twelve synthetic rainfall observations; research use rights only.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByRole("spinbutton", { name: "Price (USD)" })).toHaveValue(12.34);
    expectSplit("$11.73", "$0.61");
    expect(screen.getByRole("button", { name: "Publish to Marketplace" })).toBeEnabled();
  });
});
