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
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        invoke: async (command: string) => {
          if (command === "dashboard") return data;
          if (command === "check_updates") {
            await new Promise((resolve) => setTimeout(resolve, 250));
            return { statuses: data.statuses, failures: 0 };
          }
          if (command === "install_font") {
            await new Promise((resolve) => setTimeout(resolve, 250));
            return null;
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
  await dialog.getByRole("button", { name: "安装或更新" }).click();

  await expect(page.getByTestId("operation-status")).toContainText("正在安装 Maple Mono");
  await expect(dialog.getByRole("button", { name: "正在安装…" })).toBeDisabled();
  await expect(page.getByText("安装完成")).toBeVisible();
  await expect(page.getByTestId("operation-status")).toBeHidden();
});
