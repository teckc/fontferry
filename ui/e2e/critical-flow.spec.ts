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
          id: "nf-cn",
          name: "Nerd Font 中文版",
          description: "含中文和图标",
          assetPattern: "^MapleMono-NF-CN\\.zip$",
          default: true,
        },
        {
          id: "nl",
          name: "无连字",
          description: "关闭连字",
          assetPattern: "^MapleMonoNL\\.zip$",
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
          throw new Error(`unexpected command: ${command}`);
        },
      },
    });
  }, dashboard);
});

test("opens the catalog and lets the user choose variants", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "仪表盘" })).toBeVisible();

  await page.getByRole("button", { name: /字体目录/ }).click();
  await expect(page.getByText("Maple Mono", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: /Maple Mono/ }).click();

  const dialog = page.getByRole("dialog", { name: "Maple Mono" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Nerd Font 中文版")).toBeChecked();
  await dialog.getByLabel("无连字").check();
  await expect(dialog.getByLabel("无连字")).toBeChecked();
});
