import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("invokes the self-test only on click and displays its platform result", async () => {
  vi.mocked(invoke).mockResolvedValue({ status: "OK", version: "0.1.0", os: "macos", arch: "aarch64" });
  render(<App />);
  expect(screen.getByText(/Version 0.1.0/)).toBeInTheDocument();
  expect(invoke).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent('"status": "OK"'));
  expect(screen.getByRole("status")).toHaveTextContent("aarch64");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("self_test");
});

it("reports an IPC failure and lets the user retry", async () => {
  vi.mocked(invoke).mockRejectedValue("Self-test unavailable");
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Self-test unavailable");
  expect(screen.getByRole("button")).toBeEnabled();
});

it("disables the button while the command is pending", async () => {
  let complete!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  expect(screen.getByRole("button", { name: "Running…" })).toBeDisabled();
  complete({ status: "OK", version: "0.1.0", os: "windows", arch: "x86_64" });
  await waitFor(() => expect(screen.getByRole("button")).toBeEnabled());
});
