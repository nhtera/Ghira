// SPDX-License-Identifier: Apache-2.0
/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { bundledPackages } from "../desktop/build/bundled-packages";

// `tauri ios dev` serves the frontend to the phone over the LAN (TAURI_DEV_HOST).
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss(), bundledPackages()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    include: ["src/**/*.test.{ts,tsx}"],
    environment: "happy-dom",
    passWithNoTests: true,
  },
});
