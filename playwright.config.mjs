// @ts-check
/** Faz 28 (C) — Playwright smoke yapılandırması (yalnız smoke dosyası). */
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testMatch: "playwright-smoke.mjs",
  timeout: 45_000,
  retries: 0,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:3000",
    headless: true,
    viewport: { width: 1440, height: 900 },
  },
  reporter: [["list"]],
});
