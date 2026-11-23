import { render, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";

import App from "./App.svelte";

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class { constructor(public onmessage: (value: unknown) => void) {} },
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

const preview = { version: "2.0", identity: "confirmed", assetNames: ["font.zip"] };

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
  expect(screen.getByRole("button", { name: /安装字体|更新字体|更改字体包|重新安装/ })).toBeDisabled();
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

test("saves an optional GitHub proxy and can restore the default network", async () => {
  vi.mocked(invoke).mockResolvedValue({ fonts: [], installed: [], statuses: [], activities: [] });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  const port = screen.getByRole("spinbutton", { name: "本机 HTTP 代理端口" });
  const save = screen.getByRole("button", { name: "保存代理设置" });
  expect(port).toHaveValue(null);
  expect(save).toBeDisabled();

  await fireEvent.input(port, { target: { value: "7890" } });
  await fireEvent.click(save);
  expect(invoke).toHaveBeenCalledWith("set_github_proxy_port", { port: 7890 });
  expect(await screen.findByText("代理设置已保存，重启字渡后生效。")).toBeInTheDocument();

  await fireEvent.input(port, { target: { value: "" } });
  await fireEvent.click(save);
  expect(invoke).toHaveBeenLastCalledWith("set_github_proxy_port", { port: null });
});

test("open font details refresh after installation without reopening", async () => {
  const font = catalog.fonts[0];
  let version = "1.0";
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "prepare_font_install") return preview;
    if (command === "install_font") {
      version = "2.0";
      return { installed: { fontId: font.id, version, variantIds: [font.variants[0].id], previous: null }, families: [font.name], cleanupPending: null };
    }
    return {
      fonts: [font],
      installed: [{ fontId: font.id, version, variantIds: [font.variants[0].id], previous: null }],
      statuses: [{ fontId: font.id, currentVersion: version, availableVersion: "2.0", updateAvailable: version !== "2.0", deliveryPolicy: "autoInstall" }],
      activities: [],
    };
  });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "Aa字体" }));
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  expect(screen.getByRole("dialog")).toHaveTextContent("1.0");
  await fireEvent.click(screen.getByRole("button", { name: /安装字体|更新字体|更改字体包|重新安装/ }));
  await fireEvent.click(await screen.findByRole("button", { name: "确认安装" }));
  await waitFor(() => expect(screen.getByRole("dialog")).not.toHaveTextContent("1.0"));
  expect(screen.getByRole("dialog")).toHaveTextContent("2.0");
});


test("legacy empty variants resolve defaults only when opening details", async () => {
  const font = { ...catalog.fonts[0], variants: [{ id: "default", name: "默认包", description: "默认", assetPattern: "a", default: true }] };
  vi.mocked(invoke).mockResolvedValue({ fonts: [font], installed: [{ fontId: font.id, version: "1.0", variantIds: [], previous: null }], statuses: [], activities: [] });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "Aa字体" }));
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  const checkbox = screen.getByRole("checkbox", { name: /默认包/ });
  expect(checkbox).toBeChecked();
  await fireEvent.click(checkbox);
  expect(screen.getByRole("button", { name: /安装字体|更新字体|更改字体包|重新安装/ })).toBeDisabled();
});


test("pending scheduler operation is displayed as unknown after reload", async () => {
  vi.mocked(invoke).mockResolvedValue({ fonts: [], installed: [], statuses: [], activities: [], scheduleEnabled: null });
  render(App);
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  expect(screen.getByRole("status")).toHaveTextContent("实际状态尚未确认");
  expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
});

async function openCatalog() {
  await waitFor(() => expect(screen.queryByText("正在读取字体状态…")).not.toBeInTheDocument());
  await fireEvent.click(screen.getByRole("button", { name: "Aa字体" }));
}

test("filters management status together with search and clears empty results", async () => {
  const first = { ...catalog.fonts[0], id: "alpha", name: "Alpha" };
  const second = { ...catalog.fonts[0], id: "beta", name: "Beta" };
  vi.mocked(invoke).mockResolvedValue({ fonts: [second, first], installed: [{ fontId: "alpha", version: "1", variantIds: [], previous: null }], statuses: [], activities: [] });
  render(App);
  await openCatalog();
  await fireEvent.change(screen.getByLabelText("管理状态"), { target: { value: "managed" } });
  expect(screen.getByRole("button", { name: /Alpha/ })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Beta/ })).not.toBeInTheDocument();
  await fireEvent.input(screen.getByLabelText("搜索字体"), { target: { value: "Beta" } });
  expect(screen.getByText("没有符合条件的字体")).toBeInTheDocument();
  await fireEvent.click(screen.getByRole("button", { name: "清除筛选" }));
  expect(screen.getByRole("button", { name: /Beta/ })).toBeInTheDocument();
});

test("requires real license consent and resets it when details reopen", async () => {
  const font = { ...catalog.fonts[0], license: { ...catalog.fonts[0].license, requiresAcceptance: true } };
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "prepare_font_install") return preview;
    if (command === "install_font") return {
      installed: { fontId: font.id, version: preview.version, variantIds: [font.variants[0].id] },
      families: [font.name], cleanupPending: null,
    };
    return { fonts: [font], installed: [], statuses: [], activities: [] };
  });
  render(App);
  await openCatalog();
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  expect(screen.getByRole("button", { name: "安装字体" })).toBeDisabled();
  await fireEvent.click(screen.getByRole("checkbox", { name: "我已阅读并同意此字体的许可协议" }));
  expect(screen.getByRole("button", { name: "安装字体" })).toBeEnabled();
  await fireEvent.click(screen.getByRole("button", { name: "关闭" }));
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  expect(screen.getByRole("button", { name: "安装字体" })).toBeDisabled();
  await fireEvent.click(screen.getByRole("checkbox", { name: "我已阅读并同意此字体的许可协议" }));
  await fireEvent.click(screen.getByRole("button", { name: "安装字体" }));
  await fireEvent.click(await screen.findByRole("button", { name: "确认安装" }));
  expect(invoke).toHaveBeenCalledWith("install_font", { input: expect.objectContaining({ acceptLicense: true, fontId: font.id, candidate: preview.identity }), progress: expect.anything() });
});

test("disables removal and rollback while installation is pending", async () => {
  const font = { ...catalog.fonts[0], license: { ...catalog.fonts[0].license, requiresAcceptance: false } };
  const record = { fontId: font.id, version: "1", variantIds: [font.variants[0].id], previous: { version: "0.9" }, ownedFiles: ["sample.ttf"] };
  const report = { installed: record, families: [font.name], cleanupPending: null };
  let finish: (result: typeof report) => void = () => {};
  const pending = new Promise<typeof report>((resolve) => { finish = resolve; });
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "prepare_font_install") return preview;
    if (command === "install_font") return pending;
    return { fonts: [font], installed: [record], statuses: [], activities: [] };
  });
  render(App);
  await openCatalog();
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  await fireEvent.click(screen.getByRole("button", { name: "重新安装" }));
  await fireEvent.click(await screen.findByRole("button", { name: "确认安装" }));
  expect(screen.getByRole("button", { name: "恢复至 0.9" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "卸载字渡管理的文件" })).toBeDisabled();
  finish(report);
  await waitFor(() => expect(screen.getByRole("button", { name: "重新安装" })).toBeEnabled());
});

test("download failure preserves confirmation and offers a route to network settings", async () => {
  const font = catalog.fonts.find((item) => item.id === "smiley-sans")!;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "prepare_font_install") return preview;
    if (command === "install_font") {
      (args as { progress: { onmessage: (phase: string) => void } }).progress.onmessage("download");
      throw { kind: "network", message: "代理未连接" };
    }
    return { fonts: [font], installed: [], statuses: [], activities: [] };
  });
  render(App);
  await fireEvent.click(await screen.findByRole("button", { name: "浏览字体" }));
  await fireEvent.click(screen.getByRole("button", { name: /得意黑/ }));
  await fireEvent.click(screen.getByRole("button", { name: "安装字体" }));
  await fireEvent.click(await screen.findByRole("button", { name: "确认安装" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("下载字体失败：代理未连接");
  expect(screen.getByRole("button", { name: "确认安装" })).toBeEnabled();
  await fireEvent.click(screen.getByRole("button", { name: "打开网络设置" }));
  expect(screen.getByRole("heading", { name: "GitHub 网络代理" })).toBeInTheDocument();
  await fireEvent.click(screen.getByRole("button", { name: "返回字体详情" }));
  expect(screen.getByRole("dialog", { name: "得意黑" })).toBeInTheDocument();
});

test("failed removal keeps details and shows the failure in the drawer", async () => {
  const font = catalog.fonts[0];
  vi.spyOn(window, "confirm").mockReturnValue(true);
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "uninstall_font") throw new Error("file is busy");
    return { fonts: [font], installed: [{ fontId: font.id, version: "1", variantIds: [font.variants[0].id], previous: null }], statuses: [], activities: [] };
  });
  render(App);
  await openCatalog();
  await fireEvent.click(screen.getByRole("button", { name: new RegExp(font.name) }));
  await fireEvent.click(screen.getByRole("button", { name: "卸载字渡管理的文件" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("file is busy");
  expect(screen.getByRole("button", { name: "卸载字渡管理的文件" })).toBeEnabled();
  vi.mocked(window.confirm).mockRestore();
});

test("restores saved beta channel after UI remount", async () => {
  vi.mocked(invoke).mockResolvedValue({ fonts: [], installed: [], statuses: [], activities: [], updateChannel: "beta" });
  const first = render(App);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("dashboard"));
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  await waitFor(() => expect(screen.getByRole("combobox", { name: "程序更新通道" })).toHaveValue("beta"));
  first.unmount();
  render(App);
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  await waitFor(() => expect(screen.getByRole("combobox", { name: "程序更新通道" })).toHaveValue("beta"));
});

test("missing saved channel defaults to stable", async () => {
  vi.mocked(invoke).mockResolvedValue({ fonts: [], installed: [], statuses: [], activities: [] });
  render(App);
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  await waitFor(() => expect(screen.getByRole("combobox", { name: "程序更新通道" })).toHaveValue("stable"));
});

test("channel selection saves before remount without checking for updates", async () => {
  let saved = "stable";
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    if (command === "set_update_channel") { saved = args.channel; return; }
    return { fonts: [], installed: [], statuses: [], activities: [], updateChannel: saved };
  });
  const first = render(App);
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  const select = screen.getByRole("combobox", { name: "程序更新通道" });
  await fireEvent.change(select, { target: { value: "beta" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_update_channel", { channel: "beta" }));
  await waitFor(() => expect(select).not.toBeDisabled());
  first.unmount();
  render(App);
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  await waitFor(() => expect(screen.getByRole("combobox", { name: "程序更新通道" })).toHaveValue("beta"));
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "check_app_update")).toBe(false);
});

test("failed channel save restores previous choice", async () => {
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === "set_update_channel") throw new Error("channel save failed");
    return { fonts: [], installed: [], statuses: [], activities: [], updateChannel: "stable" };
  });
  render(App);
  await fireEvent.click(screen.getByRole("button", { name: "⚙设置" }));
  const select = screen.getByRole("combobox", { name: "程序更新通道" });
  await fireEvent.change(select, { target: { value: "beta" } });
  await waitFor(() => expect(select).toHaveValue("stable"));
  expect(await screen.findByRole("alert")).toHaveTextContent("channel save failed");
});
