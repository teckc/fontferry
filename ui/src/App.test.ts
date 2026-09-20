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

import { invoke } from "@tauri-apps/api/core";
import { cleanup, fireEvent, waitFor } from "@testing-library/svelte";
import { afterEach } from "vitest";
import catalog from "../../catalog/builtin/catalog.json";

afterEach(() => { cleanup(); vi.clearAllMocks(); });

test("restores installed variants rather than catalog defaults", async () => {
  const font = { ...catalog.fonts[0], variants: [
    { id: "default", name: "默认包", description: "默认", assetPattern: "a", default: true },
    { id: "chosen", name: "已选包", description: "已选", assetPattern: "b", default: false },
  ] };
  vi.mocked(invoke).mockResolvedValue({ fonts: [font], installed: [{ fontId: font.id, version: "1.0", variantIds: ["chosen"], previous: null }], statuses: [], activities: [] });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "Aa字体" }));
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  expect(screen.getByRole("checkbox", { name: /已选包/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /默认包/ })).not.toBeChecked();
  await fireEvent.click(screen.getByRole("checkbox", { name: /已选包/ }));
  expect(screen.getByRole("button", { name: "安装或更新" })).toBeDisabled();
});

test("failed schedule change restores persisted checkbox state", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "set_schedule") throw new Error("scheduler unavailable");
    return { fonts: [], installed: [], statuses: [], activities: [], scheduleEnabled: false };
  });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  const checkbox = screen.getByRole("checkbox");
  expect(checkbox).not.toBeChecked();
  await fireEvent.click(checkbox);
  await fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("scheduler unavailable");
  expect(checkbox).not.toBeChecked();
});
