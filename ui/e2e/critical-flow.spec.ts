import { expect, test } from "@playwright/test";

const dashboard = {
  fonts: [
    {
      id: "maple-mono",
      name: "Maple Mono",
      description: "圆角等宽字体",
      homepage: "https://github.com/subframe7536/maple-font",
      license: {
        name: "SIL Open Font License 1.1",
        url: "https://openfontlicense.org",
        spdx: "OFL-1.1",
        revision: "OFL-1.1",
        requiresAcceptance: false,
        redistributionAllowed: true,
      },
      versionProvider: { type: "githubRelease" },
      artifactProvider: { type: "githubAsset" },
      deliveryPolicy: "autoInstall",
      versionPolicy: {},
      variants: [
        {
          id: "maplemono-nf-cn",
          name: "MapleMono-NF-CN",
          description: "上游原始文件名",
          assetPattern: "^MapleMono-NF-CN\\.zip$",
          default: true,
        },
        {
          id: "maplemononl-nf-cn",
          name: "MapleMonoNL-NF-CN",
          description: "上游原始文件名",
          assetPattern: "^MapleMonoNL-NF-CN\\.zip$",
          default: false,
        },
      ],
      platforms: ["windows", "macos", "linux"],
    },
  ],
  installed: [],
  statuses: [
    {
      fontId: "maple-mono",
      currentVersion: null,
      availableVersion: "7.9",
      updateAvailable: false,
      deliveryPolicy: "autoInstall",
    },
  ],
  activities: [],
};

test.beforeEach(async ({ page }) => {
  await page.addInitScript((data) => {
    const callbacks = new Map<number, (value: unknown) => void>();
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        transformCallback: (callback: (value: unknown) => void) => {
          const id = callbacks.size + 1;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args: { progress: { id: number } }) => {
          if (command === "dashboard") return data;
          if (command === "check_updates") {
            await new Promise((resolve) => setTimeout(resolve, 250));
            return { statuses: data.statuses, failures: [] };
          }
          if (command === "prepare_font_install") {
            return { version: "7.9", assetNames: ["MapleMono-NF-CN.zip"], identity: "approved" };
          }
          if (command === "install_font") {
            const callback = callbacks.get(args.progress.id);
            callback?.({ index: 0, message: "download" });
            await new Promise((resolve) => setTimeout(resolve, 250));
            callback?.({ index: 1, end: true });
            return {
              installed: { fontId: "maple-mono", version: "7.9", variantIds: ["maplemono-nf-cn"] },
              families: ["Maple Mono"],
              cleanupPending: null,
            };
          }
          throw new Error(`unexpected command: ${command}`);
        },
      },
    });
  }, dashboard);
});

test("opens the catalog and lets the user choose variants", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "概览" })).toBeVisible();

  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Aa\s*字体/ })
    .click();
  await expect(page.getByText("Maple Mono", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: /Maple Mono/ }).click();

  const dialog = page.getByRole("dialog", { name: "Maple Mono" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("MapleMono-NF-CN")).toBeChecked();
  await dialog.getByLabel("MapleMonoNL-NF-CN").check();
  await expect(dialog.getByLabel("MapleMonoNL-NF-CN")).toBeChecked();
});

test("shows progress while checking font updates", async ({ page }) => {
  await page.goto("/");

  const checkButton = page.getByRole("button", { name: "检查字体更新" });
  await checkButton.click();

  await expect(page.getByTestId("operation-status")).toContainText("正在检查所有字体");
  await expect(page.getByRole("button", { name: "正在检查…" })).toBeDisabled();
  await expect(page.getByText("字体更新检查完成")).toBeVisible();
  await expect(page.getByTestId("operation-status")).toBeHidden();
});

test("shows progress while downloading and installing a font", async ({ page }) => {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Aa\s*字体/ })
    .click();
  await page.getByRole("button", { name: /Maple Mono/ }).click();

  const dialog = page.getByRole("dialog", { name: "Maple Mono" });
  await dialog.getByRole("button", { name: /安装字体|更新字体|更改字体包|重新安装/ }).click();
  await expect(dialog.getByText("MapleMono-NF-CN.zip")).toBeVisible();
  await dialog.getByRole("button", { name: "确认安装" }).click();

  await expect(page.getByTestId("operation-status")).toContainText("正在安装 Maple Mono");
  await expect(page.getByTestId("operation-status")).toContainText("下载字体");
  await expect(dialog.getByRole("button", { name: "正在安装…" })).toBeDisabled();
  await expect(dialog.getByText("安装完成：7.9 · Maple Mono")).toBeVisible();
  await expect(page.getByTestId("operation-status")).toBeHidden();
});

test("filters the catalog and clears an empty result", async ({ page }, testInfo) => {
  await page.goto("/");
  await page.getByRole("navigation").getByRole("button", { name: /Aa\s*字体/ }).click();
  await page.getByLabel("管理状态").selectOption("managed");
  await expect(page.getByText("没有符合条件的字体")).toBeVisible();
  await page.getByRole("button", { name: "清除筛选" }).click();
  await expect(page.getByRole("button", { name: /Maple Mono/ })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("catalog.png"), fullPage: true });
  await page.getByRole("button", { name: /Maple Mono/ }).click();
  await page.screenshot({ path: testInfo.outputPath("font-details.png"), fullPage: true });
});
