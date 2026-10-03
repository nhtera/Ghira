// SPDX-License-Identifier: Apache-2.0
// Mobile UI e2e (16-H/I/J specs; 16-K extends): the app's Vite build on the
// scripted mock, under the production CSP, in WebKit at iPhone size.
// Each slice owns its own spec files in tests/e2e; shared helpers live in
// tests/e2e/helpers.ts.
import { defineConfig, devices } from "@playwright/test";

// PORT lets parallel runs (agents, CI jobs) each have their own server.
const port = Number(process.env.PORT ?? 4183);
const origin = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "tests/e2e",
  use: { baseURL: origin },
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.01 } },
  projects: [
    {
      name: "ios-webkit",
      use: { ...devices["iPhone 15 Pro"], browserName: "webkit", viewport: { width: 390, height: 844 } },
    },
  ],
  webServer: {
    command: `PORT=${port} node tests/e2e/serve-dist.mjs`,
    url: origin,
    reuseExistingServer: !process.env.CI,
  },
});
