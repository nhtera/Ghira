// SPDX-License-Identifier: Apache-2.0

// The landing page's interactions and the privacy promise, on the built site:
// language switch (demo, notes and labels, persisted), Pause, the reduced
// motion end state, citations (transcript highlight on desktop, inline quote
// on a phone), the notes view switch, the Mac screen tabs (arrow keys), the
// comparison stacking on a phone, Copy, and: every request goes to the site
// itself, no cookie is set, and the page reads without JavaScript.

import assert from "node:assert/strict";
import { test } from "node:test";
import { LANG_KEY, openPage, siteFixture } from "./helpers.mjs";

const site = siteFixture();
const LANGS = { en: "English", vi: "Tiếng Việt" };

test("the page is complete without JavaScript", async () => {
  const ctx = await site.browser.newContext({ javaScriptEnabled: false });
  const page = await ctx.newPage();
  await page.goto(`${site.base}/`);
  const text = await page.locator("main").innerText();
  for (const s of ["Meeting notes that stay on your Mac.", "00:27", "Questions people ask first", "How it compares", "Try it today"]) assert.ok(text.includes(s), s);
  assert.ok((await page.locator(".faq details").count()) >= 6);
  assert.ok((await page.locator(".compare tbody tr").count()) >= 5);
  await ctx.close();
});

test("the meeting language switch updates the demo, notes and labels, and persists", async () => {
  const { page, context, errors } = await openPage(site, "/");
  const demoSwitch = page.locator(".demo-foot .lang-switch");
  await demoSwitch.getByRole("button", { name: LANGS.vi }).click();
  await page.waitForFunction(() => document.querySelector("#demo .rec-state")?.textContent?.includes("Đang ghi"));
  assert.match(await page.locator("#demo .privacy-pill").innerText(), /Chỉ trên máy này/);
  assert.match(await page.locator("#after .notes").innerText(), /Tóm tắt/);
  // Both switches stay in step.
  assert.equal(await page.locator("#after .lang-switch").first().getByRole("button", { name: LANGS.vi }).getAttribute("aria-pressed"), "true");
  assert.equal(await page.evaluate((k) => localStorage.getItem(k), LANG_KEY), "vi");
  await page.reload();
  await page.waitForFunction(() => document.querySelector("#demo .rec-state")?.textContent?.includes("Đang ghi"));
  assert.deepEqual(errors, []);
  await context.close();
});

test("the demo plays, pauses, and stops when paused", async () => {
  const { page, context } = await openPage(site, "/", { reducedMotion: "no-preference" });
  const clock = page.locator("#demo .rec-state .mono");
  const t0 = await clock.innerText();
  await page.waitForFunction((t) => document.querySelector("#demo .rec-state .mono")?.textContent !== t, t0);
  const pause = page.locator(".demo-foot .link-btn");
  await pause.click();
  assert.equal(await pause.getAttribute("aria-pressed"), "true");
  const paused = await clock.innerText();
  await page.waitForTimeout(800);
  assert.equal(await clock.innerText(), paused);
  await context.close();
});

test("reduced motion: the finished meeting, no Pause button", async () => {
  const { page, context } = await openPage(site, "/", { reducedMotion: "reduce" });
  await page.waitForFunction(() => document.querySelector("#demo .rec-state .mono")?.textContent === "01:41");
  assert.equal(await page.locator(".demo-foot .link-btn").count(), 0);
  await context.close();
});

test("desktop: a citation highlights its line in the transcript panel", async () => {
  const { page, context } = await openPage(site, "/", { width: 1280 });
  const chips = page.locator("#after .cite");
  assert.equal(await chips.locator("xpath=.").filter({ has: page.locator("xpath=self::*[@aria-pressed='true']") }).count(), 1);
  const second = chips.nth(1);
  await second.click();
  assert.equal(await second.getAttribute("aria-pressed"), "true");
  assert.ok((await page.locator('#after .tx-scroll [data-cited="true"]').count()) >= 1);
  assert.equal(await page.locator("#after .quote").count(), 0);
  await context.close();
});

test("phone: a citation shows the quoted line under the note", async () => {
  const { page, context } = await openPage(site, "/", { width: 375, height: 812, touch: true });
  await page.locator("#after .cite").first().tap();
  await page.locator("#after .quote").first().waitFor();
  assert.ok((await page.locator("#after .quote .quote-who").count()) >= 1);
  await context.close();
});

test("the notes view switches between what you typed and the full notes", async () => {
  const { page, context } = await openPage(site, "/");
  await page.getByRole("button", { name: "What you typed" }).click();
  await page.locator("#after .typed-pad").waitFor();
  assert.equal(await page.locator("#after .notes-group").count(), 0);
  await page.getByRole("button", { name: "Full notes" }).click();
  await page.locator("#after .notes-group").first().waitFor();
  await context.close();
});

test("Mac screen tabs: click and arrow keys switch the panel and caption", async () => {
  const { page, context } = await openPage(site, "/");
  const tabs = page.getByRole("tab");
  assert.equal(await tabs.count(), 2);
  const caption = page.locator(".mac-caption");
  const before = await caption.innerText();
  await tabs.first().focus();
  await page.keyboard.press("ArrowRight");
  assert.equal(await tabs.nth(1).getAttribute("aria-selected"), "true");
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("role")), "tab");
  assert.notEqual(await caption.innerText(), before);
  assert.equal(await page.locator('[role="tabpanel"]:not([hidden])').count(), 1);
  await page.keyboard.press("ArrowLeft");
  assert.equal(await tabs.first().getAttribute("aria-selected"), "true");
  await context.close();
});

test("phone: the comparison is one block per question, nothing scrolls sideways", async () => {
  const { page, context } = await openPage(site, "/", { width: 375, height: 812, touch: true });
  assert.equal(await page.locator(".compare tbody tr").first().evaluate((el) => getComputedStyle(el).display), "block");
  assert.equal(await page.locator(".compare thead").evaluate((el) => el.getBoundingClientRect().width <= 1), true);
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  await context.close();
});

test("Copy: clipboard when allowed, else the code is selected", async () => {
  const allowed = await openPage(site, "/");
  await allowed.context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: site.base });
  const btn = allowed.page.locator("#get .copy-btn");
  await btn.click();
  await assert.doesNotReject(allowed.page.waitForFunction(() => document.querySelector("#get .copy-btn")?.textContent === "Copied"));
  assert.match(await allowed.page.evaluate(() => navigator.clipboard.readText()), /git clone https:\/\/github\.com\/nhtera\/Ghira\.git/);
  await allowed.context.close();

  const denied = await openPage(site, "/");
  await denied.page.evaluate(() => {
    Object.defineProperty(navigator, "clipboard", { value: { writeText: () => Promise.reject(new Error("denied")) } });
  });
  await denied.page.locator("#get .copy-btn").click();
  await denied.page.waitForFunction(() => document.querySelector("#get .copy-btn")?.textContent === "Selected");
  assert.match(await denied.page.evaluate(() => String(getSelection())), /git clone/);
  await denied.context.close();
});

test("privacy: every request goes to the site itself, and no cookie is set", async () => {
  const ctx = await site.browser.newContext({ reducedMotion: "no-preference" });
  const page = await ctx.newPage();
  const origin = new URL(site.base).origin;
  const foreign = [];
  const cookies = [];
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (u.protocol !== "data:" && u.origin !== origin) foreign.push(r.url());
  });
  page.on("response", async (r) => {
    const h = await r.allHeaders();
    if (h["set-cookie"]) cookies.push(r.url());
  });
  for (const path of ["/", "/docs", "/docs/privacy", "/nope"]) {
    await page.goto(site.base + path);
    await page.waitForLoadState("networkidle");
  }
  await page.goto(`${site.base}/`);
  await page.locator(".demo-foot .lang-switch").getByRole("button", { name: LANGS.vi }).click();
  await page.locator("#after .cite").nth(2).click();
  await page.getByRole("tab").nth(1).click();
  // Load every screenshot the page shows (the hidden theme's are display:none).
  await page.locator("#apps").scrollIntoViewIfNeeded();
  await page.locator(".phones").scrollIntoViewIfNeeded();
  await page.waitForLoadState("networkidle");
  assert.deepEqual(foreign, []);
  assert.deepEqual(cookies, []);
  assert.equal(await page.evaluate(() => document.cookie), "");
  assert.deepEqual(await ctx.cookies(), []);
  await ctx.close();
});
