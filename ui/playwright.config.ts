import path from "node:path";

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        channel: process.env.FONTFERRY_E2E_CHANNEL,
      },
    },
  ],
  webServer: {
    command: "pnpm dev -- --host 127.0.0.1",
    cwd: path.resolve(import.meta.dirname, ".."),
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
  },
});
