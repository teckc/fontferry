import { render, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";

import App from "./App.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({
    fonts: [],
    installed: [],
    statuses: [],
    activities: [],
  }),
}));

test("renders the primary navigation", async () => {
  render(App);
  expect(await screen.findByText("字体")).toBeInTheDocument();
  expect(screen.getByText("添加字体")).toBeInTheDocument();
  expect(screen.getByText("记录")).toBeInTheDocument();
});
