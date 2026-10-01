// SPDX-License-Identifier: Apache-2.0
// The component gallery: its own Vite root, so stories never reach the app
// bundle (the app uses a few mocks on its mock core only). `pnpm --filter @ghi/ui gallery` (dev) / `gallery:build`.
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  base: "./",
  plugins: [react(), tailwindcss()],
  server: { port: 6006, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true },
});
