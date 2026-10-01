// SPDX-License-Identifier: Apache-2.0
// RT-6: with the production CSP, the webview cannot reach any remote origin.
// Scope: this checks the policy from tauri.conf.json in real browser engines.
// Tauri adds nonces/hashes when it serves the app, which only tightens it.
// Top-level navigation is outside CSP; the Rust navigation guard covers it.
import { expect, test } from "@playwright/test";

test("CSP blocks remote images, fetches and sockets", async ({ page }) => {
  let remoteHits = 0;
  await page.route(/^https?:\/\/(?!127\.0\.0\.1)/, (route) => {
    remoteHits += 1;
    return route.abort();
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();

  const violations = await page.evaluate(async () => {
    const seen: string[] = [];
    document.addEventListener("securitypolicyviolation", (e) => seen.push(`${e.effectiveDirective} ${e.blockedURI}`));

    const img = document.createElement("img");
    img.src = "https://example.com/pixel.png";
    document.body.append(img);
    await new Promise((resolve) => {
      img.onerror = resolve;
      img.onload = resolve;
    });

    const fetched = await fetch("https://example.com/").then(
      () => "allowed",
      () => "blocked",
    );
    let socket = "blocked";
    try {
      new WebSocket("wss://example.com/");
      socket = "allowed";
    } catch {
      /* CSP throws synchronously in some engines; others report a violation */
    }
    const form = document.createElement("form");
    form.action = "https://example.com/collect";
    form.method = "post";
    document.body.append(form);
    form.submit();

    await new Promise((resolve) => setTimeout(resolve, 300));
    return { seen, fetched, socket };
  });

  expect(violations.fetched).toBe("blocked");
  expect(violations.seen.some((v) => v.startsWith("img-src https://example.com"))).toBe(true);
  expect(violations.seen.some((v) => v.startsWith("connect-src https://example.com"))).toBe(true);
  expect(violations.seen.some((v) => v.startsWith("connect-src wss://example.com"))).toBe(true);
  expect(violations.seen.some((v) => v.startsWith("form-action"))).toBe(true);
  expect(remoteHits).toBe(0);
});

test("CSP blocks remote stylesheets (inline styles are allowed, remote ones not)", async ({ page }) => {
  await page.route(/^https?:\/\/(?!127\.0\.0\.1)/, (route) => route.abort());
  await page.goto("/");
  const seen = await page.evaluate(async () => {
    const out: string[] = [];
    document.addEventListener("securitypolicyviolation", (e) => out.push(`${e.effectiveDirective} ${e.blockedURI}`));
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = "https://example.com/x.css";
    document.head.append(link);
    await new Promise((r) => setTimeout(r, 300));
    return out;
  });
  expect(seen.some((v) => v.startsWith("style-src-elem https://example.com") || v.startsWith("style-src https://example.com"))).toBe(true);
});
