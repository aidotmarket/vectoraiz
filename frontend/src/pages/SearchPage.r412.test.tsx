import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import SearchPage from "./SearchPage";

// Mount the actual page, with no app/bootstrap or real API imports. Native select
// presentation makes the chosen value and the actual search options observable.
const h = vi.hoisted(() => ({
  param: "A" as string | null,
  navigate: vi.fn(), search: vi.fn(),
  data: undefined as { datasets: { id: string; original_filename: string; status: string }[] } | undefined,
  loading: false, listError: null as string | null,
  searching: false, results: null as { results: []; total: number } | null,
  searchError: null as string | null,
  presentation: async (names: string[]) => {
    const { createElement } = await import("react");
    return Object.fromEntries(names.map(name => [name,
      ({ children }: { children?: ReactNode }) => createElement("div", null, children),
    ]));
  },
}));
vi.mock("react-router-dom", () => ({
  useNavigate: () => h.navigate,
  useSearchParams: () => [new URLSearchParams(h.param === null ? "" : { dataset: h.param })],
}));
vi.mock("@/hooks/useApi", () => ({
  useDatasets: () => ({ data: h.data, loading: h.loading, error: h.listError }),
  useSearch: () => ({ search: h.search, results: h.results, loading: h.searching, error: h.searchError }),
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock("@/components/ui/input", () => ({ Input: (props: React.InputHTMLAttributes<HTMLInputElement>) => <input {...props} /> }));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick, disabled }: { children: ReactNode; onClick?: () => void; disabled?: boolean }) =>
    <button onClick={onClick} disabled={disabled}>{children}</button>,
}));
vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, children }: { value: string; onValueChange: (value: string) => void; children: ReactNode }) =>
    <select aria-label="Select Dataset" value={value} onChange={e => onValueChange(e.target.value)}>{children}</select>,
  SelectContent: ({ children }: { children: ReactNode }) => <>{children}</>,
  SelectItem: ({ value, children }: { value: string; children: ReactNode }) => <option value={value}>{children}</option>,
  SelectTrigger: () => null, SelectValue: () => null,
}));
vi.mock("@/components/ui/card", () => h.presentation(["Card", "CardContent"]));
vi.mock("@/components/ui/alert", () => h.presentation(["Alert", "AlertDescription"]));
vi.mock("@/components/ui/progress", () => h.presentation(["Progress"]));
vi.mock("@/components/ui/skeleton", () => h.presentation(["Skeleton"]));
vi.mock("@/components/ui/dialog", () => ({
  Dialog: () => null, DialogContent: () => null, DialogHeader: () => null, DialogTitle: () => null,
}));

const dataset = (id: string, status = "ready") => ({ id, original_filename: `${id}.csv`, status });
const list = (...ids: string[]) => ({ datasets: ids.map(id => dataset(id)) });
const selection = () => (screen.getByRole("combobox") as HTMLSelectElement).value;
const choose = (value: string) => fireEvent.change(screen.getByRole("combobox"), { target: { value } });
const typeQuery = (value = "synthetic query") => fireEvent.change(screen.getByRole("textbox"), { target: { value } });
const clickSearch = () => fireEvent.click(screen.getByRole("button", { name: "Search", exact: true }));

beforeEach(() => {
  vi.clearAllMocks();
  h.param = "A"; h.data = list("A", "B"); h.loading = false; h.listError = null;
  h.searching = false; h.results = null; h.searchError = null;
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.stubGlobal("XMLHttpRequest", vi.fn(() => { throw new Error("Unexpected network"); }));
});
afterEach(() => {
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  cleanup(); vi.unstubAllGlobals();
});

it("initial URL A selects A without an automatic search", () => {
  render(<SearchPage />);
  expect(selection()).toBe("A");
  expect(h.search).not.toHaveBeenCalled();
});

it.each(["B", "__all__"])("manual %s survives typing, examples, refresh and search lifecycle", async choice => {
  const view = render(<SearchPage />);
  choose(choice);
  expect(selection()).toBe(choice);
  typeQuery();
  fireEvent.click(screen.getByRole("button", { name: "Find customers from California" }));
  expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe("Find customers from California");
  h.data = list("A", "B"); view.rerender(<SearchPage />);
  expect(selection()).toBe(choice);
  expect(h.search).not.toHaveBeenCalled();

  let finish!: () => void;
  h.search.mockImplementationOnce(() => {
    h.searching = true;
    view.rerender(<SearchPage />);
    return new Promise<void>(resolve => { finish = resolve; });
  });
  clickSearch();
  expect(h.search).toHaveBeenCalledTimes(1);
  expect(h.search).toHaveBeenCalledWith("Find customers from California", choice === "__all__" ? {} : { dataset_id: "B" });
  expect(selection()).toBe(choice);
  expect((screen.getByRole("button", { name: "Search", exact: true }) as HTMLButtonElement).disabled).toBe(true);
  h.searching = false; h.results = { results: [], total: 0 };
  finish(); await Promise.resolve(); view.rerender(<SearchPage />);
  expect(selection()).toBe(choice);
  expect(screen.getByText("No results found")).toBeTruthy();
  expect(h.search).toHaveBeenCalledTimes(1);
});

// Assert the invoked search scope directly so the causal base comparison also
// demonstrates the wrong API options, rather than stopping at the select value.
it.each(["B", "__all__"])("one click searches the manually chosen %s scope", choice => {
  render(<SearchPage />);
  choose(choice); typeQuery(); clickSearch();
  expect(h.search).toHaveBeenCalledExactlyOnceWith("synthetic query", choice === "__all__" ? {} : { dataset_id: "B" });
});

it("late list initializes the URL default once", () => {
  h.data = undefined; h.loading = true;
  const view = render(<SearchPage />);
  expect(h.search).not.toHaveBeenCalled();
  h.data = list("A", "B"); h.loading = false; view.rerender(<SearchPage />);
  expect(selection()).toBe("A");
  choose("B"); h.data = list("A", "B"); view.rerender(<SearchPage />);
  expect(selection()).toBe("B");
  expect(h.search).not.toHaveBeenCalled();
});

it.each(["B", "__all__"])("late arrival of URL dataset preserves explicit %s", choice => {
  h.data = list("B");
  const view = render(<SearchPage />);
  choose(choice);
  h.loading = true; view.rerender(<SearchPage />);
  h.data = list("A", "B"); h.loading = false; view.rerender(<SearchPage />);
  expect(selection()).toBe(choice);
  typeQuery(); clickSearch();
  expect(h.search).toHaveBeenCalledExactlyOnceWith("synthetic query", choice === "__all__" ? {} : { dataset_id: "B" });
});

it("a URL dataset becoming available initializes when there was no manual override", () => {
  h.data = list("B");
  const view = render(<SearchPage />);
  expect(selection()).toBe("__all__");
  h.data = list("A", "B"); view.rerender(<SearchPage />);
  expect(selection()).toBe("A");
  expect(h.search).not.toHaveBeenCalled();
});

it("navigation applies new URL defaults, including A to B to A", () => {
  const view = render(<SearchPage />);
  choose("__all__");
  h.param = "B"; view.rerender(<SearchPage />);
  expect(selection()).toBe("B");
  choose("__all__");
  h.param = "A"; view.rerender(<SearchPage />);
  expect(selection()).toBe("A");
  choose("B"); view.rerender(<SearchPage />);
  expect(selection()).toBe("B");
  expect(h.search).not.toHaveBeenCalled();
});

it.each([null, "", "unknown"])("URL %s keeps normal All and single dataset choices", param => {
  h.param = param;
  const view = render(<SearchPage />);
  expect(selection()).toBe("__all__");
  expect(screen.queryByRole("button", { name: /^Back to/ })).toBeNull();
  typeQuery(); clickSearch();
  expect(h.search).toHaveBeenCalledExactlyOnceWith("synthetic query", {});
  choose("B"); h.data = list("A", "B"); view.rerender(<SearchPage />);
  expect(selection()).toBe("B");
  h.search.mockClear(); clickSearch();
  expect(h.search).toHaveBeenCalledExactlyOnceWith("synthetic query", { dataset_id: "B" });
});

it.each(["B", "__all__"])("Back label and origin target agree after manual %s", choice => {
  const view = render(<SearchPage />);
  choose(choice);
  expect(selection()).toBe(choice);
  fireEvent.click(screen.getByRole("button", { name: "Back to A.csv" }));
  expect(h.navigate).toHaveBeenCalledExactlyOnceWith("/datasets/A");
  h.param = "B"; view.rerender(<SearchPage />);
  h.navigate.mockClear();
  fireEvent.click(screen.getByRole("button", { name: "Back to B.csv" }));
  expect(h.navigate).toHaveBeenCalledExactlyOnceWith("/datasets/B");
  expect(h.search).not.toHaveBeenCalled();
});

it("empty, list error and non-ready datasets keep their existing presentation", () => {
  h.data = list();
  const view = render(<SearchPage />);
  expect(screen.getByText("No datasets available. Upload a dataset first.")).toBeTruthy();
  h.listError = "synthetic failure"; view.rerender(<SearchPage />);
  expect(screen.getByText("Failed to load datasets: synthetic failure")).toBeTruthy();
  h.listError = null; h.data = { datasets: [dataset("A", "processing"), dataset("B")] };
  view.rerender(<SearchPage />);
  expect(selection()).toBe("__all__");
  expect(screen.queryByRole("option", { name: "A.csv" })).toBeNull();
  expect(h.search).not.toHaveBeenCalled();
});
