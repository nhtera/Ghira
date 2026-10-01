// SPDX-License-Identifier: Apache-2.0
// Serves a built frontend with the production CSP header, like the Tauri
// asset protocol does, so Playwright can check that the policy is enforced.
// ROOT (default: the app's dist) and PORT (default 4173) pick what and where;
// the component gallery is served the same way.
import { createServer } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = process.env.ROOT ? join(process.env.ROOT, "/") : fileURLToPath(new URL("../../dist/", import.meta.url));
const conf = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
const csp = conf.app.security.csp;
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".json": "application/json" };
const port = Number(process.env.PORT ?? 4173);

createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^([/\\])+/, "");
  let file = join(root, path || "index.html");
  if (!file.startsWith(root) || !existsSync(file)) file = join(root, "index.html");
  res.writeHead(200, { "Content-Type": types[extname(file)] ?? "application/octet-stream", "Content-Security-Policy": csp });
  res.end(readFileSync(file));
}).listen(port, "127.0.0.1", () => console.log(`serving dist on http://127.0.0.1:${port}`));
