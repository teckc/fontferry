import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

test("historical result and stable-release fallback remain available", async ({ page }) => {
  const catalog = JSON.parse(readFileSync(new URL("../../catalog/builtin/catalog.json", import.meta.url), "utf8"));
  const fonts = catalog.fonts.filter((font: { id: string }) => ["smiley-sans", "maple-mono"].includes(font.id));
  await page.addInitScript((fonts) => {
    const stale = { fontId: "smiley-sans", currentVersion: null, availableVersion: "v2.0.1",
      updateAvailable: true, deliveryPolicy: "autoInstall", checkedAt: "2026-11-22T00:00:00Z", fromCache: true };
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: { invoke: async (command: string) => {
      if (command === "dashboard") return { fonts, installed: [], activities: [], statuses: [stale] };
      if (command === "check_updates") return {
        statuses: [{ ...stale, fontId: "maple-mono", availableVersion: "7.9", fromCache: false }],
        failures: ["smiley-sans"],
      };
      if (command === "check_app_update") throw "更新通道暂无可用清单，请前往发行页获取安装包";
      throw new Error(command);
    } } });
  }, fonts);
  await page.goto("/");
  await page.getByRole("button", { name: "检查字体更新" }).click();
  await expect(page.getByText("检查完成，1 个来源检查失败：得意黑")).toBeVisible();
  await page.getByRole("button", { name: /得意黑/ }).click();
  const drawer = page.getByRole("dialog", { name: "得意黑" });
  await expect(drawer.getByText("上次查到的版本")).toBeVisible();
  await expect(drawer.getByText(/缓存的远程版本信息/)).toBeVisible();
  await drawer.getByRole("button", { name: "关闭" }).click();
  await page.getByRole("navigation").getByRole("button", { name: /设置/ }).click();
  await page.getByRole("button", { name: "检查程序更新" }).click();
  await expect(page.getByRole("alert")).toContainText("更新通道暂无可用清单");
  await expect(page.getByRole("link", { name: "前往发行页手动下载安装包" }))
    .toHaveAttribute("href", "https://github.com/teckc/fontferry/releases");
});
