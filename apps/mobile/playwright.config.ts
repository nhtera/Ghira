// SPDX-License-Identifier: Apache-2.0
// Mobile UI e2e (16-H/I/J specs; 16-K extends): the app's Vite build on the
// scripted mock, under the production CSP, in WebKit at iPhone size.
// Each slice owns its own spec files in tests/e2e; shared helpers live in
// tests/e2e/helpers.ts.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/e2e",
  use: { baseURL: "http://127.0.0.1:4183" },
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01 } },
  projects: [
    {
      name: "ios-webkit",
      use: { ...devices["iPhone 15 Pro"], browserName: "webkit", viewport: { width: 390, height: 844 } },
    },
  ],
  webServer: {
    command: "node tests/e2e/serve-dist.mjs",
    url: "http://127.0.0.1:4183",
    reuseExistingServer: !process.env.CI,
  },
});
