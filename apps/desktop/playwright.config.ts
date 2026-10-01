// SPDX-License-Identifier: Apache-2.0
// One entry point (`pnpm --filter @ghi/desktop test:e2e`) for:
// - e2e: the app's Vite build with the mocked core, prod CSP, chromium + webkit;
// - gallery-csp: every component story under the prod CSP, zero violations;
// - gallery-a11y: axe on every story, light/dark × en/vi (chromium);
// - gallery-visual: one screenshot per story × theme × language, webkit on
//   macOS only (WKWebView is what ships there; CI runs Linux).
import { defineConfig, devices } from "@playwright/test";

const gallery = "http://127.0.0.1:4174";

export default defineConfig({
  testDir: "tests/e2e",
  use: { baseURL: "http://127.0.0.1:4173" },
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01 } },
  projects: [
    { name: "chromium", testIgnore: /gallery/, use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", testIgnore: /gallery/, use: { ...devices["Desktop Safari"] } },
    { name: "gallery-csp-chromium", testMatch: /gallery-csp/, use: { ...devices["Desktop Chrome"], baseURL: gallery } },
    { name: "gallery-csp-webkit", testMatch: /gallery-csp/, use: { ...devices["Desktop Safari"], baseURL: gallery } },
    // axe is injected as a script: bypass the CSP for it (the CSP has its own project).
    { name: "gallery-a11y", testMatch: /gallery-a11y/, use: { ...devices["Desktop Chrome"], baseURL: gallery, bypassCSP: true } },
    {
      name: "gallery-visual",
      testMatch: /gallery-visual/,
      use: { ...devices["Desktop Safari"], baseURL: gallery, viewport: { width: 1280, height: 900 } },
    },
  ],
  webServer: [
    {
      command: "node tests/e2e/serve-dist.mjs",
      url: "http://127.0.0.1:4173",
      reuseExistingServer: !process.env.CI,
    },
    {
      command: "node tests/e2e/serve-dist.mjs",
      env: { ROOT: "../../packages/ui/gallery/dist", PORT: "4174" },
      url: gallery,
      reuseExistingServer: !process.env.CI,
    },
  ],
});
