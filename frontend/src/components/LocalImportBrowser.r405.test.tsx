import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { importApi, type ImportBrowseEntry, type ImportBrowseResponse } from "@/lib/api";
import { LocalImportBrowser } from "./LocalImportBrowser";

vi.mock("@/lib/api", () => ({
  importApi: { browse: vi.fn(), start: vi.fn(), scan: vi.fn(), getStatus: vi.fn(), cancel: vi.fn() },
}));
vi.mock("@/contexts/BrandContext", () => ({
  useBrand: () => ({ importDir: "/synthetic/imports", importDirEnvVar: "VECTORAIZ_IMPORT_DIR" }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() } }));

const root = "/synthetic/imports/";
const file: ImportBrowseEntry = { name: "retail.csv", type: "file", size_bytes: 12 };
const folder: ImportBrowseEntry = { name: "nested", type: "directory" };
const listing = (entries: ImportBrowseEntry[] = [], path = root): ImportBrowseResponse => ({
  path, entries, total: entries.length, limit: 500, offset: 0,
});
const browse = vi.mocked(importApi.browse);
const retry = () => fireEvent.click(screen.getByRole("button", { name: "Retry" }));
const expectError = async (message: string) => {
  expect(await screen.findByRole("alert")).toHaveTextContent("Unable to read import directory.");
  expect(screen.getByText(message, { exact: true })).toBeInTheDocument();
  expect(screen.queryByText("No files found in import directory.")).toBeNull();
  expect(screen.queryByText(/Add files to/)).toBeNull();
};

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network request"); }));
});
afterEach(() => {
  cleanup();
  for (const method of [importApi.start, importApi.scan, importApi.cancel, importApi.getStatus]) {
    expect(method).not.toHaveBeenCalled();
  }
  expect(fetch).not.toHaveBeenCalled();
  vi.unstubAllGlobals();
});

describe("R405 local import directory read retry", () => {
  it("shows the exact initial read error without empty advice or automatic retry", async () => {
    browse.mockRejectedValue(new Error("Synthetic folder service unavailable"));
    render(<LocalImportBrowser />);
    await expectError("Synthetic folder service unavailable");
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(browse.mock.calls).toEqual([[""]]);
  });

  it("retries the initial request, initializes root breadcrumbs, and allows navigation and selection", async () => {
    browse.mockRejectedValueOnce(new Error("Synthetic initial failure"))
      .mockResolvedValueOnce(listing([file, folder]))
      .mockResolvedValueOnce(listing([file], root + "nested/"))
      .mockResolvedValueOnce(listing([file, folder]));
    render(<LocalImportBrowser />);
    await expectError("Synthetic initial failure");
    retry();
    fireEvent.click(await screen.findByRole("button", { name: "nested" }));
    expect(await screen.findByRole("button", { name: "imports" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "nested" })).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /retail.csv/ }));
    expect(screen.getByRole("button", { name: "Import Selected (1)" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "imports" }));
    expect(await screen.findByRole("button", { name: "Select All" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import Selected (0)" })).toBeDisabled();
    expect(browse.mock.calls).toEqual([[""], [""], [root + "nested/"], [root]]);
  });

  it("clears the error and shows genuine empty advice after a successful empty retry", async () => {
    browse.mockRejectedValueOnce(new Error("Synthetic read failure")).mockResolvedValueOnce(listing());
    render(<LocalImportBrowser />);
    await expectError("Synthetic read failure");
    retry();
    expect(await screen.findByText("No files found in import directory.")).toBeInTheDocument();
    expect(screen.getByText(/Add files to/)).toHaveTextContent("/synthetic/imports");
    expect(screen.getByText(/Add files to/)).toHaveTextContent("VECTORAIZ_IMPORT_DIR");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(screen.getByRole("button", { name: "imports" })).toBeInTheDocument();
    expect(browse.mock.calls).toEqual([[""], [""]]);
  });

  it("retries the actual failed subfolder through repeated failures rather than the previous successful folder", async () => {
    browse.mockResolvedValueOnce(listing([folder]))
      .mockRejectedValueOnce(new Error("Synthetic subfolder failure"))
      .mockRejectedValueOnce(new Error("Synthetic repeated failure"))
      .mockResolvedValueOnce(listing([file], root + "nested/"));
    render(<LocalImportBrowser />);
    fireEvent.click(await screen.findByRole("button", { name: "nested" }));
    await expectError("Synthetic subfolder failure");
    retry();
    await expectError("Synthetic repeated failure");
    retry();
    expect(await screen.findByText("retail.csv")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "imports" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "nested" })).toBeInTheDocument();
    expect(browse.mock.calls).toEqual([[""], ...Array.from({ length: 3 }, () => [root + "nested/"])]);
  });

  it("hides Retry while its read is pending so it cannot be clicked again", async () => {
    let resolveRead!: (value: ImportBrowseResponse) => void;
    browse.mockRejectedValueOnce(new Error("Synthetic read failure"))
      .mockImplementationOnce(() => new Promise((resolve) => { resolveRead = resolve; }));
    render(<LocalImportBrowser />);
    await expectError("Synthetic read failure");
    retry();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("No files found in import directory.")).toBeNull();
    expect(screen.getByRole("button", { name: "Import Selected (0)" })).toBeDisabled();
    expect(browse.mock.calls).toEqual([[""], [""]]);
    await act(async () => { resolveRead(listing([file])); });
    expect(screen.getByText("retail.csv")).toBeInTheDocument();
    expect(browse).toHaveBeenCalledTimes(2);
  });

  it("preserves the fallback message for a non-Error read rejection", async () => {
    browse.mockRejectedValueOnce(null);
    render(<LocalImportBrowser />);
    await expectError("Failed to browse directory");
  });

  it("retains ordinary successful empty-directory advice", async () => {
    browse.mockResolvedValueOnce(listing());
    render(<LocalImportBrowser />);
    expect(await screen.findByText("No files found in import directory.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(browse).toHaveBeenCalledTimes(1);
  });

  it("retains ordinary file selection, select all, and deselect all without importing", async () => {
    browse.mockResolvedValueOnce(listing([file, { ...file, name: "other.csv" }]));
    render(<LocalImportBrowser />);
    fireEvent.click(await screen.findByRole("button", { name: /retail.csv/ }));
    expect(screen.getByRole("button", { name: "Import Selected (1)" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Select All" }));
    expect(screen.getByRole("button", { name: "Import Selected (2)" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Deselect All" }));
    expect(screen.getByRole("button", { name: "Import Selected (0)" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expect(browse).toHaveBeenCalledTimes(1);
  });
});
