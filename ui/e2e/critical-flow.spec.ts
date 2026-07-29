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
