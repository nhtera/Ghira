// SPDX-License-Identifier: Apache-2.0
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { bundledPackages } from "../desktop/build/bundled-packages";

// `tauri ios dev` serves the frontend to the phone over the LAN (TAURI_DEV_HOST).
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), bundledPackages()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
