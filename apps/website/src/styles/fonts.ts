// SPDX-License-Identifier: Apache-2.0

// Self-hosted fonts (OFL-1.1), the app's own (src/styles/fonts.css). The
// latin files of the body text (400, 600) and the headline serif are
// preloaded so first paint uses them. These are the same files the
// @font-face rules point at, so Vite emits one asset each: no double download.
import beVietnam400 from "../../node_modules/@fontsource/be-vietnam-pro/files/be-vietnam-pro-latin-400-normal.woff2?url";
import beVietnam600 from "../../node_modules/@fontsource/be-vietnam-pro/files/be-vietnam-pro-latin-600-normal.woff2?url";
import sourceSerif from "../../node_modules/@fontsource-variable/source-serif-4/files/source-serif-4-latin-opsz-normal.woff2?url";

export const fontPreloads = [beVietnam400, beVietnam600, sourceSerif].map((href) => ({
  rel: "preload",
  href,
  as: "font",
  type: "font/woff2",
  crossOrigin: "anonymous" as const,
}));
