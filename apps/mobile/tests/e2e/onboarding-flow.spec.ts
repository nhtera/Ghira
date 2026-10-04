// SPDX-License-Identifier: Apache-2.0
// M1 onboarding on the scripted mock: every step, the microphone denied, the
// model download failing, and resuming where the user left off.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { log, openOnboarding, openRecord, setKnobs } from "./record-support";

const next = (page: import("@playwright/test").Page) => page.getByRole("button", { name: "Continue", exact: true });
const step = (page: import("@playwright/test").Page) => page.locator("[data-screen=onboarding]");

test("walks every step and ends on Record", async ({ page }) => {
  await openOnboarding(page);

  // 1. Languages
  await expect(page.getByRole("heading", { name: "Which languages do you speak in meetings?" })).toBeVisible();
  await expect(page.getByText("Step 1 of 7")).toBeVisible();
  await expect(page.getByRole("radio", { name: "Both, often mixed" })).toBeChecked();
  await page.getByRole("radio", { name: "Tiếng Việt" }).check({ force: true });
  await next(page).click();

  // 2. Microphone: priming, then the system prompt answers
  await expect(page.getByRole("heading", { name: /needs the microphone/ })).toBeVisible();
  await expect(page.getByText("Step 2 of 7")).toBeVisible();
  await page.getByRole("button", { name: "Allow microphone" }).click();
  await expect(page.getByRole("heading", { name: "Microphone allowed" })).toBeVisible();
  await next(page).click();

  // 3. Consent explainer
  await expect(page.getByRole("heading", { name: "Tell people before you record" })).toBeVisible();
  await next(page).click();

  // 4. Processing default: this phone; My computer and Cloud are off
  await expect(page.getByRole("heading", { name: "Where should recordings be processed?" })).toBeVisible();
  await expect(page.getByRole("radio", { name: "This phone" })).toBeChecked();
  await expect(page.getByRole("radio", { name: "My computer" })).toBeDisabled();
  await expect(page.getByRole("radio", { name: "Cloud · off" })).toBeDisabled();
  await next(page).click();

  // 5. Models: Wi-Fi notice and size, then ready
  await expect(page.getByRole("heading", { name: "Download the speech models" })).toBeVisible();
  await expect(page.getByText("Downloads over Wi-Fi only.")).toBeVisible();
  await expect(page.getByText(/about 877 MB/)).toBeVisible();
  await page.getByRole("button", { name: "Download 877 MB" }).click();
  await expect(page.getByText("Models are ready")).toBeVisible();
  await expect(page.locator("li[data-state=ready]")).toHaveCount(3);
  await next(page).click();

  // 6. Voice (optional, after the models it needs): consent first, the passage after it
  await expect(page.getByRole("heading", { name: /Your voice/ })).toBeVisible();
  await expect(page.getByText("Read this aloud")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Start reading" })).toHaveCount(0);
  await page.getByRole("checkbox", { name: /I agree to store a voice profile/ }).check();
  await expect(page.getByText(/In our weekly sync we review the roadmap/)).toBeVisible();
  await page.getByRole("button", { name: "Start reading" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Listening…" })).toBeVisible();
  await setKnobs(page, { voiceSeconds: 16 });
  await page.getByRole("button", { name: "Done reading" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Saved" })).toBeVisible();
  await next(page).click();

  // 7. Done
  await expect(page.getByRole("heading", { name: "You’re set" })).toBeVisible();
  await expect(page.getByText("Step 7 of 7")).toBeVisible();
  await page.getByRole("button", { name: "Start recording" }).click();
  await expect(page.locator("[data-screen=record]")).toBeVisible();
  expect(new URL(page.url()).hash).toBe("#/record");
  expect(await log(page)).toContain("requestMicPermission");
});

test("steps back without losing the way forward", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages"] });
  await expect(page.getByText("Step 2 of 7")).toBeVisible();
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByRole("heading", { name: "Which languages do you speak in meetings?" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Back" })).toHaveCount(0);
});

test("a bar per step shows progress; a swipe to the right goes back, a swipe left does nothing", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming"] });
  const bars = page.getByTestId("onboarding-progress").locator("span");
  await expect(bars).toHaveCount(7);
  await expect(page.getByTestId("onboarding-progress").locator("[data-done=true]")).toHaveCount(3);
  const swipe = (dx: number) =>
    page.getByTestId("onboarding-page").evaluate((el, d) => {
      const fire = (type: string, x: number) => {
        const e = new Event(type, { bubbles: true });
        const t = [{ clientX: x, clientY: 300 }];
        Object.defineProperty(e, "touches", { value: type === "touchend" ? [] : t });
        Object.defineProperty(e, "changedTouches", { value: t });
        el.dispatchEvent(e);
      };
      fire("touchstart", 100);
      fire("touchend", 100 + d);
    }, dx);
  await swipe(-120);
  await expect(step(page)).toHaveAttribute("data-step", "consent");
  await swipe(120);
  await expect(step(page)).toHaveAttribute("data-step", "micPriming");
});

test("resumes at the first step not completed", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming"] });
  await expect(step(page)).toHaveAttribute("data-step", "consent");
  await expect(page.getByText("Step 3 of 7")).toBeVisible();
});

test("the record tab sends a first launch to onboarding", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages"] });
  await page.evaluate(() => (window.location.hash = "#/record"));
  await expect(step(page)).toBeVisible();
  await expect(page.locator("[data-screen=record]")).toHaveCount(0);
});

test("microphone denied: Settings or on without it", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages"], knobs: { micAnswer: "denied" } });
  await page.getByRole("button", { name: "Allow microphone" }).click();
  await expect(page.getByText(/Microphone is off for .*Settings → .* → Microphone\./)).toBeVisible();
  await expect(page.getByTestId("mic-denied")).toBeVisible();
  await page.getByRole("button", { name: "Open Settings" }).click();
  expect(await log(page)).toContain("openAppSettings");

  // Back from Settings with the switch on: the step notices.
  await setKnobs(page, { mic: "granted" });
  await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
  await expect(page.getByRole("heading", { name: "Microphone allowed" })).toBeVisible();
});

test("microphone denied: Not now moves on", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages"], knobs: { micAnswer: "denied" } });
  await page.getByRole("button", { name: "Allow microphone" }).click();
  await page.getByRole("button", { name: "Not now" }).click();
  await expect(step(page)).toHaveAttribute("data-step", "consent");
});

test("voice: skip leaves no consent behind, and waits for the voice model", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing", "models"] });
  await page.getByRole("checkbox", { name: /I agree/ }).check();
  await page.getByRole("button", { name: "Skip" }).click();
  await expect(step(page)).toHaveAttribute("data-step", "done");

  await setKnobs(page, { voiceModel: false });
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByText(/once the speech models are downloaded/)).toBeVisible();
  await expect(page.getByRole("checkbox", { name: /I agree/ })).toBeDisabled();
});

test("voice: a failed save says so and lets the user retry or skip", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing", "models"] });
  await page.getByRole("checkbox", { name: /I agree/ }).check();
  await setKnobs(page, { voiceStartError: "storage" });
  await page.getByRole("button", { name: "Start reading" }).click();
  await expect(page.getByText("Couldn’t save your voice. Try again, or skip.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Skip" })).toBeVisible();
});

test("model download error: the progress is kept and Try again works", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { failModels: true } });
  await page.getByRole("button", { name: "Download 877 MB" }).click();
  await expect(page.getByText("The download stopped. What was already downloaded is kept.")).toBeVisible();
  await expect(page.locator("li[data-state=failed]")).toHaveCount(1);
  await expect(next(page)).toHaveCount(0);

  await setKnobs(page, { failModels: false });
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByText("Models are ready")).toBeVisible();
  await expect(page.getByText("The download stopped.")).toHaveCount(0);
});

test("models: waiting for Wi-Fi offers cellular once; later leaves the models out", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { onCellular: true } });
  await expect(page.getByText(/Waiting for Wi-Fi\. The download continues/)).toBeVisible();
  await page.getByRole("button", { name: "Download over cellular this time" }).click();
  expect(await log(page)).toContain("modelsDownload:false");

  // The mock's download finishes at once: start again from the waiting state.
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { onCellular: true } });
  await page.getByRole("button", { name: "Download later" }).click();
  await expect(step(page)).toHaveAttribute("data-step", "voice");
});

test("models already on the phone: nothing to download", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { modelsReady: true } });
  await expect(page.getByText("Models are ready")).toBeVisible();
  await expect(page.getByRole("button", { name: /^Download/ })).toHaveCount(0);
});

test("a record-only phone cannot pick This phone", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent"], knobs: { tier: "recordOnly" } });
  await expect(page.getByRole("radio", { name: "This phone" })).toBeDisabled();
});

test("Vietnamese copy", async ({ page }) => {
  await openOnboarding(page, { lang: "vi" });
  await expect(page.getByRole("heading", { name: "Bạn thường họp bằng ngôn ngữ nào?" })).toBeVisible();
  await expect(page.getByText("Bước 1 / 7")).toBeVisible();
  await page.getByRole("button", { name: "Tiếp tục" }).click();
  await expect(page.getByRole("button", { name: "Cho phép micro" })).toBeVisible();
});

const STEPS: { name: string; completed: string[] }[] = [
  { name: "languages", completed: [] },
  { name: "mic priming", completed: ["languages"] },
  { name: "voice", completed: ["languages", "micPriming"] },
  { name: "consent", completed: ["languages", "micPriming", "voice"] },
  { name: "processing", completed: ["languages", "micPriming", "consent"] },
  { name: "models", completed: ["languages", "micPriming", "consent", "processing"] },
  { name: "done", completed: ["languages", "micPriming", "voice", "consent", "processing", "models"] },
];

for (const lang of ["en", "vi"] as const) {
  for (const s of STEPS) {
    test(`axe: ${s.name} (${lang})`, async ({ page }) => {
      await openOnboarding(page, { lang, completed: s.completed });
      await expect(page.locator("[data-step-title]")).toBeVisible();
      await expectAccessible(page);
    });
  }
}

test("axe: voice with consent and listening", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing", "models"] });
  await page.getByRole("checkbox", { name: /I agree/ }).check();
  await expectAccessible(page);
  await page.getByRole("button", { name: "Start reading" }).click();
  await expect(page.getByText("Listening…")).toBeVisible();
  await expectAccessible(page);
});

test("axe: mic denied", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages"], knobs: { micAnswer: "denied" } });
  await page.getByRole("button", { name: "Allow microphone" }).click();
  await expect(page.getByRole("button", { name: "Open Settings" })).toBeVisible();
  await expectAccessible(page);
});

test("axe: download error", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { failModels: true } });
  await page.getByRole("button", { name: "Download 877 MB" }).click();
  await expect(page.getByText(/The download stopped/)).toBeVisible();
  await expectAccessible(page);
});

test("a record-only phone is told the models are optional", async ({ page }) => {
  await openOnboarding(page, { completed: ["languages", "micPriming", "consent", "processing"], knobs: { tier: "recordOnly" } });
  await expect(page.getByText("This phone records only, so the speech models are optional.")).toBeVisible();
  await expectAccessible(page);
});

test("the done step summarizes the setup", async ({ page }) => {
  await openOnboarding(page, { completed: [...["languages", "micPriming", "consent", "processing", "models"], "voice"], knobs: { modelsReady: true } });
  const summary = page.locator("[data-step=done] ul");
  await expect(summary).toContainText("Languages");
  await expect(summary).toContainText("Both, often mixed");
  await expect(summary).toContainText("This phone");
  await expect(summary.getByText("Ready")).toBeVisible();
  await expect(summary).toContainText("Not added");
  await expectAccessible(page);
});

test("a setup that cannot be read says so and retries, never starting over silently", async ({ page }) => {
  await openOnboarding(page, { completed: [], knobs: { failOnboarding: "disk I/O error" } });
  await expect(step(page)).toHaveAttribute("data-step", "error");
  await expect(page.getByText("Couldn’t load your setup. Try again.")).toBeVisible();
  await expect(page.locator("[data-step-title]")).toHaveCount(0);
  await expectAccessible(page);
  await setKnobs(page, { failOnboarding: null });
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("heading", { name: "Which languages do you speak in meetings?" })).toBeVisible();
});

test("a locked store does not send the Record tab to onboarding; a broken one does", async ({ page }) => {
  await openRecord(page);
  await setKnobs(page, { failOnboarding: "locked" });
  await page.evaluate(() => (window.location.hash = "#/search"));
  await page.evaluate(() => (window.location.hash = "#/record"));
  await expect(page.locator("[data-screen=record]")).toBeVisible();
  await setKnobs(page, { failOnboarding: "disk I/O error" });
  await page.evaluate(() => (window.location.hash = "#/search"));
  await page.evaluate(() => (window.location.hash = "#/record"));
  await expect(step(page)).toBeVisible();
});
