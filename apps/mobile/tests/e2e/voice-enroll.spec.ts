// SPDX-License-Identifier: Apache-2.0
// "Me" voice enrollment on the phone (onboarding step and Settings → Voice):
// the level meter, the timer, "Done reading" waiting for 15 s, the automatic
// finish at 25 s and a message for each way a try can fail.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";
import { openOnboarding, setKnobs } from "./record-support";

const PRE = ["languages", "micPriming", "consent", "processing", "models"];

async function onboardingReading(page: Page, knobs: Record<string, unknown> = {}) {
  await openOnboarding(page, { completed: PRE, knobs: { voiceSeconds: 5, ...knobs } });
  await page.getByRole("checkbox", { name: /I agree/ }).check();
  await page.getByRole("button", { name: "Start reading" }).click();
}

async function settingsReading(page: Page, knobs: Record<string, unknown> = {}) {
  await openApp(page, "/settings/voice");
  await setKnobs(page, { voiceSeconds: 5, ...knobs });
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Set up my voice" }).click();
}

const screens = [
  { name: "onboarding", reading: onboardingReading, done: "Done reading", saved: (p: Page) => expect(p.getByRole("status").filter({ hasText: "Saved" })).toBeVisible() },
  { name: "settings", reading: settingsReading, done: "Done reading", saved: (p: Page) => expect(p.getByText("Voice profile saved.")).toBeVisible() },
] as const;

for (const s of screens) {
  test.describe(s.name, () => {
    test("shows a level meter and the timer; the button waits for 15 s", async ({ page }) => {
      await s.reading(page);
      const meter = page.getByRole("meter", { name: "Voice level" });
      await expect(meter).toBeVisible();
      await expect(meter).toHaveAttribute("aria-valuemin", "0");
      await expect(meter).toHaveAttribute("aria-valuemax", "100");
      await expect(meter).toHaveAttribute("aria-valuenow", /^\d+$/);
      await expect(page.getByTestId("enroll-timer")).toHaveText("Reading… 5 s of about 25 s");
      await expect(page.getByText("Keep reading. You can finish after 15 s.")).toBeVisible();
      await expect(page.getByRole("button", { name: s.done })).toBeDisabled();
      await expectAccessible(page);

      await setKnobs(page, { voiceSeconds: 15 });
      await expect(page.getByTestId("enroll-timer")).toHaveText("Reading… 15 s of about 25 s");
      await expect(page.getByRole("button", { name: s.done })).toBeEnabled();
      await expect(page.getByText(/Keep reading/)).toHaveCount(0);
      await page.getByRole("button", { name: s.done }).click();
      await s.saved(page);
    });

    test("a silent mic says so in words, not only in color", async ({ page }) => {
      await s.reading(page, { voiceLevel: 0 });
      await expect(page.getByRole("meter")).toHaveAttribute("aria-valuetext", "Can’t hear you. Hold the phone closer.");
      await expect(page.getByText("Can’t hear you. Hold the phone closer.")).toBeVisible();
      await setKnobs(page, { voiceLevel: 0.6 });
      await expect(page.getByRole("meter")).toHaveAttribute("aria-valuetext", "Hearing you");
      await expect(page.getByRole("meter")).toHaveAttribute("aria-valuenow", "60");
    });

    test("finishes by itself when the buffer is full", async ({ page }) => {
      await s.reading(page);
      await setKnobs(page, { voiceSeconds: 25 });
      await s.saved(page);
      await expect(page.getByRole("meter")).toHaveCount(0);
    });

    for (const [code, text] of [
      ["tooShort", /That was too short\. Read the whole passage, about 20 seconds/],
      ["tooQuiet", /Hold the phone closer and try again/],
      ["micPermission", /Microphone access is off\. Turn it on in Settings/],
      ["noModel", /once the speech models are downloaded/],
      ["storage", s.name === "settings" ? /Couldn’t save your voice\. Try again\.$/ : /Couldn’t save your voice\. Try again, or skip\./],
    ] as const) {
      test(`a ${code} failure says what to do`, async ({ page }) => {
        await s.reading(page, { voiceStopError: code, voiceSeconds: 16 });
        await page.getByRole("button", { name: s.done }).click();
        await expect(page.getByText(text).first()).toBeVisible();
        await expect(page.getByRole("meter")).toHaveCount(0);
      });
    }
  });
}

test("a start without microphone access says how to allow it", async ({ page }) => {
  await openOnboarding(page, { completed: PRE, knobs: { voiceStartError: "micPermission" } });
  await page.getByRole("checkbox", { name: /I agree/ }).check();
  await page.getByRole("button", { name: "Start reading" }).click();
  await expect(page.getByText(/Microphone access is off\. Turn it on in Settings/)).toBeVisible();
});

test("Vietnamese: the timer and the longer passage", async ({ page }) => {
  await openOnboarding(page, { completed: PRE, lang: "vi", knobs: { voiceSeconds: 5 } });
  await page.getByRole("checkbox").check();
  await expect(page.getByText(/Nếu có gì chậm trễ/)).toBeVisible();
  await page.getByRole("button", { name: "Bắt đầu đọc" }).click();
  await expect(page.getByTestId("enroll-timer")).toHaveText("Đang đọc… 5/25 giây");
  await expect(page.getByRole("button", { name: "Đọc xong" })).toBeDisabled();
});
