// SPDX-License-Identifier: Apache-2.0
import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

const NETWORK_MESSAGE = "Network I/O goes through Rust (ghi-net), not the webview.";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "src/bindings.ts", "test-results", "playwright-report"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": reactHooks },
    rules: reactHooks.configs.recommended.rules,
  },
  {
    // RT-6: the webview never does network I/O itself; everything goes through
    // Rust commands and ghi-net. The CSP enforces this too.
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-globals": [
        "error",
        ...["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "RTCPeerConnection", "webkitRTCPeerConnection"].map(
          (name) => ({ name, message: NETWORK_MESSAGE }),
        ),
      ],
      "no-restricted-properties": [
        "error",
        ...["window", "globalThis", "self"].flatMap((object) =>
          ["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "RTCPeerConnection", "open"].map((property) => ({
            object,
            property,
            message: NETWORK_MESSAGE,
          })),
        ),
        { object: "navigator", property: "sendBeacon", message: NETWORK_MESSAGE },
      ],
    },
  },
);
