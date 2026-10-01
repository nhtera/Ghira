// SPDX-License-Identifier: Apache-2.0
// Lint rules shared by every frontend (apps/*, packages/*): no network I/O
// from the webview, safe rendering (RT-6), and no hard-coded UI strings.
import i18next from "eslint-plugin-i18next";

const NETWORK_MESSAGE = "Network I/O goes through Rust (ghi-net), not the webview.";
const NETWORK_APIS = ["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "RTCPeerConnection", "webkitRTCPeerConnection"];

/** RT-6: the webview never does network I/O itself; the CSP enforces it too. */
export const noNetwork = {
  rules: {
    "no-restricted-globals": ["error", ...NETWORK_APIS.map((name) => ({ name, message: NETWORK_MESSAGE }))],
    "no-restricted-properties": [
      "error",
      ...["window", "globalThis", "self"].flatMap((object) =>
        [...NETWORK_APIS, "open"].map((property) => ({ object, property, message: NETWORK_MESSAGE })),
      ),
      { object: "navigator", property: "sendBeacon", message: NETWORK_MESSAGE },
    ],
  },
};

/**
 * RT-6: transcript, notes, AI output and file metadata are rendered as text
 * nodes only: no raw HTML, no Markdown renderers that emit links or images.
 */
export const safeRendering = {
  rules: {
    "no-restricted-syntax": [
      "error",
      {
        selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
        message: "RT-6: render text nodes only (no raw HTML).",
      },
      {
        selector: "AssignmentExpression[left.property.name=/^(innerHTML|outerHTML)$/]",
        message: "RT-6: render text nodes only (no raw HTML).",
      },
    ],
    "no-restricted-imports": [
      "error",
      {
        paths: ["react-markdown", "marked", "markdown-it", "dompurify", "html-react-parser"].map((name) => ({
          name,
          message: "RT-6: untrusted text is never rendered as Markdown/HTML.",
        })),
      },
    ],
  },
};

/** UI copy comes from @ghi/i18n: JSX text and text-bearing attributes. */
export const noLiteralStrings = {
  plugins: { i18next },
  rules: {
    "i18next/no-literal-string": [
      "error",
      {
        mode: "jsx-only",
        "jsx-attributes": { include: ["aria-label", "aria-description", "title", "placeholder", "alt", "label"] },
        words: { exclude: ["[0-9!-/:-@[-`{-~·•…—–→←↑↓⌘⇧⌥⌃+ ]+", "[A-Z_-]+"] },
      },
    ],
  },
};
