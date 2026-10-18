import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@/hooks/use-toast";
import { piiApi, getApiUrl } from "@/lib/api";
import type { Dataset } from "@/types/mockDatasets";
import PublishModal from "./PublishModal";

vi.mock("@/hooks/use-toast", () => ({ toast: vi.fn(() => { throw new Error("Unexpected toast"); }) }));
vi.mock("@/contexts/AuthContext", () => ({ useAuth: () => ({ apiKey: "synthetic-key" }) }));
vi.mock("@/lib/api", () => ({
  piiApi: { getScan: vi.fn() },
  getApiUrl: vi.fn(() => { throw new Error("Preview must not publish"); }),
}));

const dataset: Dataset = {
  id: "synthetic-r457", name: "synthetic.csv", type: "csv", status: "ready",
  rows: 10, columns: 2, size: "1 KB", sizeBytes: 1024,
  createdAt: new Date("2026-10-01"), modifiedAt: new Date("2026-10-01"), processingTime: 0,
};
const onPublishSuccess = vi.fn(() => { throw new Error("Unexpected publish callback"); });
const onOpenChange = vi.fn(() => { throw new Error("Unexpected modal action"); });

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(piiApi.getScan).mockResolvedValue({ column_results: [] } as never);
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(() => { throw new Error("Unexpected XHR"); });
  vi.spyOn(window, "open").mockImplementation(() => { throw new Error("Unexpected external navigation"); });
  vi.stubGlobal("WebSocket", vi.fn(() => { throw new Error("Unexpected WebSocket"); }));
  for (const method of ["setItem", "removeItem", "clear"] as const) {
    vi.spyOn(Storage.prototype, method).mockImplementation(() => { throw new Error("Unexpected storage write"); });
  }
});

afterEach(() => {
  try {
    expect(fetch).not.toHaveBeenCalled();
    expect(XMLHttpRequest.prototype.open).not.toHaveBeenCalled();
    expect(window.open).not.toHaveBeenCalled();
    expect(WebSocket).not.toHaveBeenCalled();
    for (const method of ["setItem", "removeItem", "clear"] as const) {
      expect(Storage.prototype[method]).not.toHaveBeenCalled();
    }
    expect(toast).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(getApiUrl).not.toHaveBeenCalled();
    expect(onPublishSuccess).not.toHaveBeenCalled();
  } finally {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

const clean = {
  dataset_id: dataset.id, filename: "synthetic.csv", scanned_at: "2026-10-18T18:00:00",
  total_rows: 10, rows_sampled: 10, total_columns: 2, columns_with_pii: 0,
  columns_clean: 2, overall_risk: "none", privacy_score: 10, column_results: [],
  clean_columns: ["x", "y"], duration_seconds: 0.01, entities_checked: ["EMAIL_ADDRESS"],
};
const finding = (column: string, risk_level: string) => ({
  column, pii_types: ["EMAIL_ADDRESS"], risk_level,
  sample_matches: [{ value: "SYNTHETIC_PRIVATE_SAMPLE", confidence: 0.9 }],
});

function openPrivacy() {
  render(<PublishModal open onOpenChange={onOpenChange} dataset={dataset} onPublishSuccess={onPublishSuccess} />);
}
function expectUnknown() {
  // Causal checks come first: the base displays a score even when no observation exists.
  expect(screen.queryByText(/\d+(?:\.\d+)?\/10/)).not.toBeInTheDocument();
  expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  expect(screen.queryByText("Good privacy protection")).not.toBeInTheDocument();
  expect(screen.queryByText("No privacy concerns detected")).not.toBeInTheDocument();
  expect(screen.queryByText(/appears to be free|No PII was found/)).not.toBeInTheDocument();
  expect(document.querySelector(".lucide-shield-check")).toBeNull();
}
function nextAndBack() {
  expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox", { name: "I confirm I have reviewed the privacy implications" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("heading", { name: "Dataset Metadata" })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByRole("button", { name: "Next" })).toBeEnabled();
  expect(piiApi.getScan).toHaveBeenCalledExactlyOnceWith(dataset.id);
}

describe("R457 privacy observation display", () => {
  it("does not rate a pending observation and retains acknowledgement navigation", () => {
    vi.mocked(piiApi.getScan).mockReturnValue(new Promise(() => {}));
    openPrivacy();
    expectUnknown();
    expect(screen.getByText("Checking privacy scan results…")).toBeVisible();
    nextAndBack();
    expectUnknown();
  });

  it.each([new Error("Synthetic HTTP 503"), new TypeError("Synthetic transport failure"), new Error("")])(
    "does not rate rejected reads: %s", async error => {
      vi.mocked(piiApi.getScan).mockRejectedValue(error);
      openPrivacy();
      await screen.findByText("Unable to scan for PII. Proceed with caution.");
      expectUnknown();
      expect(screen.getByText("Privacy scan results are unavailable. Privacy risk is unknown.")).toBeVisible();
      nextAndBack();
      expectUnknown();
    },
  );

  it.each([
    ["cached scan_failed 200", { dataset_id: dataset.id, filename: "synthetic.csv", status: "scan_failed", error: "SYNTHETIC_PRIVATE_ERROR" }, false],
    ["cached skipped 200", { dataset_id: dataset.id, filename: "synthetic.csv", status: "skipped", reason: "no_text_content" }, true],
    ["failed with array", { status: "scan_failed", column_results: [] }, false],
    ["skipped with findings", { status: "skipped", column_results: [finding("secret", "high")] }, true],
    ["null", null, false], ["undefined", undefined, false], ["empty", {}, false],
    ["missing columns", { overall_risk: "none", privacy_score: 10 }, false],
    ["null columns", { column_results: null }, false],
    ["object columns", { column_results: {} }, false],
    ["string columns", { column_results: "SYNTHETIC_PRIVATE_SAMPLE" }, false],
    ["null column", { column_results: [null] }, false],
    ["missing types", { column_results: [{ column: "secret", risk_level: "high" }] }, false],
    ["string types", { column_results: [{ column: "secret", pii_types: "EMAIL_ADDRESS", risk_level: "high" }] }, false],
    ["object type", { column_results: [{ column: "secret", pii_types: [{}], risk_level: "high" }] }, false],
    ["bad column", { column_results: [{ ...finding("secret", "high"), column: {} }] }, false],
    ["bad risk", { column_results: [finding("secret", "unknown")] }, false],
    ["bad samples", { column_results: [{ ...finding("secret", "high"), sample_matches: {} }] }, false],
    ["mixed columns", { column_results: [finding("secret", "high"), {}] }, false],
    ["explicit pending", { status: "pending", column_results: [] }, false],
    ["stale failure marker", { scan_status: "scan_failed", column_results: [] }, false],
  ])("does not rate unusable observation %s", async (_name, payload, skipped) => {
    vi.mocked(piiApi.getScan).mockResolvedValue(payload as never);
    openPrivacy();
    await act(async () => {});
    expectUnknown();
    expect(screen.getByText(skipped
      ? "Privacy scan was skipped. Privacy risk is unknown."
      : "No usable privacy scan results are available. Privacy risk is unknown.")).toBeVisible();
    expect(screen.queryByText(/SYNTHETIC_PRIVATE/)).not.toBeInTheDocument();
    nextAndBack();
    expectUnknown();
  });

  it.each([clean, { column_results: [] }, { column_results: [{ column: "x", pii_types: [], risk_level: "none" }] }])(
    "preserves clean observations without invented scan_status or mandatory metadata %j", async payload => {
      vi.mocked(piiApi.getScan).mockResolvedValue(payload as never);
      openPrivacy();
      await screen.findByText("No privacy concerns detected");
      expect(screen.getByText("10.0/10")).toBeVisible();
      expect(screen.getByRole("progressbar")).toBeVisible();
      expect(screen.getByText("Good privacy protection")).toBeVisible();
      nextAndBack();
      expect(screen.getByText("10.0/10")).toBeVisible();
    },
  );

  it("preserves valid PII scores, exclusion toggles and document behavior without exposing sample values", async () => {
    vi.mocked(piiApi.getScan).mockResolvedValue({ ...clean, overall_risk: "high", privacy_score: 4,
      column_results: [finding("email", "high"), finding("contact", "medium"), finding("document_content", "low")],
    } as never);
    openPrivacy();
    await screen.findByText("4.0/10");
    expect(screen.getByText("Document Content")).toBeVisible();
    expect(screen.getAllByText("Contains EMAIL_ADDRESS (1 samples)")).toHaveLength(3);
    expect(screen.queryByText(/SYNTHETIC_PRIVATE/)).not.toBeInTheDocument();
    const exclusions = screen.getAllByRole("checkbox").filter(box => box.id !== "privacy-confirm");
    expect(exclusions).toHaveLength(2);
    fireEvent.click(exclusions[0]);
    expect(screen.getByText("7.0/10")).toBeVisible();
    fireEvent.click(exclusions[1]);
    expect(screen.getByText("9.0/10")).toBeVisible();
    fireEvent.click(exclusions[0]);
    expect(screen.getByText("6.0/10")).toBeVisible();
    nextAndBack();
    expect(screen.getByText("6.0/10")).toBeVisible();
  });

  it("replaces a pending read with a genuine result", async () => {
    let resolve!: (value: never) => void;
    vi.mocked(piiApi.getScan).mockReturnValue(new Promise(done => { resolve = done; }));
    openPrivacy();
    expectUnknown();
    await act(async () => resolve(clean as never));
    await waitFor(() => expect(screen.getByText("10.0/10")).toBeVisible());
  });
});
