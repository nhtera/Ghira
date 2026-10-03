// SPDX-License-Identifier: Apache-2.0
// M2 record on the scripted mock: the whole flow, the phase banners, the
// interruption and call sheets, a long transcript, and reloading mid-session.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { clipboardWrites, log, openRecord, setKnobs, startRecording, stubClipboard } from "./record-support";

const phase = (page: Page, p: string) => page.evaluate((x) => window.__ghiMock!.simulateMobileEvent({ type: "phase", phase: x as never }), p);
const mobile = (page: Page, e: object) => page.evaluate((x) => window.__ghiMock!.simulateMobileEvent(x as never), e);
const addLines = (page: Page, n: number) => page.evaluate((x) => window.__ghiRecord!.addLines(x), n);
const lines = (page: Page) => page.getByTestId("line");

async function live(page: Page) {
  await startRecording(page, "Record room");
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  // Stop shows while the models still load; wait for live before driving phases.
  await expect(page.getByText("Getting ready")).toHaveCount(0);
}

test("start -> lines -> mark -> pause -> resume -> stop", async ({ page }) => {
  await stubClipboard(page);
  await openRecord(page);

  // Idle: Phone is the target, the rest wait for pairing / the meeting view.
  await expect(page.getByRole("heading", { name: "Room recording" })).toBeVisible();
  await expect(page.getByRole("radio", { name: "This phone" })).toBeChecked();
  await expect(page.getByRole("radio", { name: "My computer" })).toBeDisabled();
  await expect(page.getByText("Audio stays on your devices.")).toBeVisible();

  // The consent reminder comes first; the message can be copied; cancel starts nothing.
  await page.getByRole("button", { name: "Record room" }).click();
  const consent = page.getByRole("dialog", { name: "Tell everyone you’re recording" });
  await expect(consent).toBeVisible();
  await consent.getByRole("button", { name: "Cancel" }).click();
  await expect(consent).toBeHidden();
  expect(await log(page)).not.toContain("recordStart");

  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("button", { name: "Copy message" }).click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  // Language "auto": the message in both languages.
  expect(await clipboardWrites(page)).toEqual(["Heads up: I’m recording this meeting for notes.\nMình báo trước: mình đang ghi âm cuộc họp này để lấy ghi chú."]);
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();

  // Live.
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  const start = await page.evaluate(() => window.__ghiRecord!.lastStart);
  expect(start).toMatchObject({ mode: "room", target: "phone", consentAcknowledged: true, callAcknowledged: false, title: null });
  await expect(page.getByText("Recording · Local only")).toBeVisible();
  await expect(page.getByText("Lines appear here as people speak.")).toBeVisible();

  await addLines(page, 4);
  await expect(lines(page)).toHaveCount(4);
  await expect(page.getByTestId("line").first()).toContainText("Speaker 1");
  await expect(page.getByTestId("line").nth(2)).toContainText("Speaker 3");
  // One polite announcement for the newest turn, not a line each.
  await expect(page.getByRole("status").filter({ hasText: /speaking/ })).toHaveText("Speaker 1 speaking");

  await page.evaluate(() => window.__ghiRecord!.addPartial("chốt scope cho"));
  await expect(page.getByTestId("partial")).toHaveText("chốt scope cho");

  await page.getByRole("button", { name: "Mark" }).click();
  await expect(page.getByText("1 mark", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark, 1 mark" })).toBeVisible();
  // The star lands on the line the mark fell on.
  await expect(lines(page).first().locator("[data-icon=star]")).toHaveCount(1);
  await expect(lines(page).nth(1).locator("[data-icon=star]")).toHaveCount(0);

  // The clock counts recorded time only.
  await page.evaluate(() => window.__ghiMock!.simulateMobileEvent({ type: "phase", phase: "paused" }));
  await expect(page.getByRole("button", { name: "Resume" }).first()).toBeVisible();
  await expect(page.getByText("Paused · Local only")).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark" })).toBeDisabled();
  await page.getByRole("button", { name: "Resume" }).first().click();
  await expect(page.getByRole("button", { name: "Pause" }).first()).toBeVisible();
  await page.getByRole("button", { name: "Pause" }).first().click();
  await expect(page.getByRole("button", { name: "Resume" }).first()).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).first().click();

  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("button", { name: "Record room" })).toBeVisible();
  await expect(page.getByText("Recording saved")).toBeVisible();
  await expect(lines(page)).toHaveCount(0);
  expect(await log(page)).toEqual(expect.arrayContaining(["recordStart", "recordMark", "recordPause", "recordResume", "recordStop"]));

  await page.getByRole("button", { name: "Open Meetings" }).click();
  expect(new URL(page.url()).hash).toBe("#/meetings");
});

test("the timer counts recorded seconds", async ({ page }) => {
  await page.clock.install();
  await openRecord(page);
  await live(page);
  await expect(page.getByText("0:00")).toBeVisible();
  await page.clock.runFor(3000);
  await expect(page.getByText("0:03")).toBeVisible();
  await phase(page, "paused");
  await page.clock.runFor(5000);
  await expect(page.getByText("0:03")).toBeVisible();
  await phase(page, "live");
  await page.clock.runFor(2000);
  await expect(page.getByText("0:05")).toBeVisible();
});

test("loading with a session is recording: the clock runs, Mark and Stop work", async ({ page }) => {
  await page.clock.install();
  await openRecord(page, { knobs: { holdLoading: true } });
  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText("Getting ready")).toBeVisible();
  // Not "Checking microphone…": the microphone is already on.
  await expect(page.getByText("Checking microphone…")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark" })).toBeEnabled();
  await expect(page.getByText("Recording · Local only")).toBeVisible();
  await page.clock.runFor(4000);
  await expect(page.getByText("0:04")).toBeVisible();
  await page.getByRole("button", { name: "Mark" }).click();
  await expect(page.getByText("1 mark", { exact: true })).toBeVisible();
  await page.evaluate(() => window.__ghiRecord!.release());
  await expect(page.getByText("Getting ready")).toHaveCount(0);
  await page.clock.runFor(1000);
  await expect(page.getByText("0:05")).toBeVisible();
});

test("without a session yet, starting shows the spinner", async ({ page }) => {
  await openRecord(page, { knobs: { failStart: "waitingForTranscription" } });
  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText("Finishing the last recording… try again in a moment.")).toBeVisible();
});

test("locked: recording continues, the transcript catches up when the app opens", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await phase(page, "locked");
  await expect(page.getByText("Recording while locked")).toBeVisible();
  await expect(page.getByText(/The transcript catches up when you open/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expectAccessible(page);

  await mobile(page, { type: "backlog", backlogS: 60, catchUpX: 2 });
  await phase(page, "catchingUp");
  await expect(page.getByText("Catching up · 0%")).toBeVisible();
  await mobile(page, { type: "backlog", backlogS: 15, catchUpX: 2 });
  await expect(page.getByText("Catching up · 75%")).toBeVisible();
  await expect(page.getByText("0:15 of audio left to transcribe.")).toBeVisible();
  await expectAccessible(page);

  await mobile(page, { type: "backlog", backlogS: 0, catchUpX: null });
  await phase(page, "live");
  await expect(page.getByText(/Catching up/)).toHaveCount(0);
  await expect(page.getByText("Recording while locked")).toHaveCount(0);
});

test("too hot, record-only and muffled sound each get their banner", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await phase(page, "hot");
  await expect(page.getByText("The phone is too hot")).toBeVisible();
  await expect(page.getByText("Transcription pauses until it cools down. Recording continues.")).toBeVisible();
  await expectAccessible(page);

  await phase(page, "recordOnly");
  await expect(page.getByText("Recording only on this phone")).toBeVisible();
  await expect(page.getByText("The phone is too hot")).toHaveCount(0);

  await phase(page, "live");
  await mobile(page, { type: "pocket", muffled: true });
  await expect(page.getByText("Sound is muffled. Is the phone in a pocket?")).toBeVisible();
  await expectAccessible(page);
  await mobile(page, { type: "pocket", muffled: false });
  await expect(page.getByText(/Sound is muffled/)).toHaveCount(0);
});

test("a phone below the live tier records only: no target choice, it records and says so", async ({ page }) => {
  await page.clock.install();
  await openRecord(page, { knobs: { tier: "recordOnly" } });
  await expect(page.getByText("Recording only on this phone — processed later")).toBeVisible();
  await expect(page.getByText("This phone is too old to transcribe. The recording is saved and processed later.")).toBeVisible();
  await expect(page.getByRole("radio")).toHaveCount(0);
  await expectAccessible(page);

  await startRecording(page);
  const start = await page.evaluate(() => window.__ghiRecord!.lastStart);
  expect(start).toMatchObject({ target: "phone", consentAcknowledged: true });
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByText("Recording only on this phone", { exact: true })).toBeVisible();
  await page.clock.runFor(3000);
  await expect(page.getByText("0:03")).toBeVisible();
  await page.getByRole("button", { name: "Mark" }).click();
  await expectAccessible(page);

  // Stopping shows Saved and stops the clock (a late recordOnly after done is covered by the reducer tests).
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByText("Recording saved")).toBeVisible();
  await page.clock.runFor(5000);
  await expect(page.getByRole("button", { name: "Record room" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
  await expect(page.getByText("Recording saved")).toBeVisible();
});

test("record-only says why, and models missing offers the download", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await setKnobs(page, { forceReason: true });
  for (const [reason, text] of [
    ["engineFailed", "Transcription stopped working. Recording continues and is processed later."],
    ["thermal", "The phone is too hot to transcribe. Recording continues."],
    ["deviceTier", "This phone is too old to transcribe."],
  ] as const) {
    await setKnobs(page, { reason });
    await phase(page, "live");
    await phase(page, "recordOnly");
    await expect(page.getByText(text)).toBeVisible();
  }
  await setKnobs(page, { reason: "modelsMissing" });
  await phase(page, "live");
  await phase(page, "recordOnly");
  await expect(page.getByText(/speech models aren’t downloaded yet/)).toBeVisible();
  await expectAccessible(page);
  await page.getByRole("button", { name: "Download models" }).click();
  expect(new URL(page.url()).hash).toBe("#/settings");
});

test("interruption by a call: the sheet asks, Resume continues", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 2);
  await mobile(page, { type: "interruption", began: true, kind: "call" });
  await phase(page, "interrupted");

  const sheet = page.getByRole("dialog", { name: "Paused for a phone call" });
  await expect(sheet).toBeVisible();
  await expect(sheet).toContainText("stopped at 0:00 so the call wasn’t recorded");
  await expectAccessible(page);

  // It does not close by itself, nor on Escape, when the call ends.
  await mobile(page, { type: "interruption", began: false, kind: "call" });
  await page.keyboard.press("Escape");
  await expect(sheet).toBeVisible();
  expect(await log(page)).not.toContain("recordResume");

  await sheet.getByRole("button", { name: "Resume recording" }).click();
  await expect(sheet).toBeHidden();
  expect(await log(page)).toContain("recordResume");
});

test("interruption by another app: Stop and save", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await mobile(page, { type: "interruption", began: true, kind: "other" });
  await phase(page, "interrupted");
  const sheet = page.getByRole("dialog", { name: "Recording paused" });
  await expect(sheet).toContainText("Another app took the microphone");
  await sheet.getByRole("button", { name: "Stop and save" }).click();
  await expect(sheet).toBeHidden();
  await expect(page.getByText("Recording saved")).toBeVisible();
  expect(await log(page)).toContain("recordStop");
});

test("M6: with a phone call active the call notice replaces the reminder", async ({ page }) => {
  await stubClipboard(page);
  await openRecord(page, { knobs: { callActive: true } });
  await page.getByRole("button", { name: "Record room" }).click();
  const sheet = page.getByRole("dialog", { name: "Phones can’t record calls" });
  await expect(sheet).toBeVisible();
  await expect(sheet).toContainText("Tell the other person you’re recording before you start.");
  await expect(page.getByRole("dialog", { name: "Tell everyone you’re recording" })).toHaveCount(0);
  await expectAccessible(page);
  await sheet.getByRole("button", { name: "Copy message" }).click();
  expect(await clipboardWrites(page)).toHaveLength(1);

  await sheet.getByRole("button", { name: "Use speakerphone and Room mode" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  const start = await page.evaluate(() => window.__ghiRecord!.lastStart);
  expect(start).toMatchObject({ callAcknowledged: true, consentAcknowledged: true });
});

test("the call block follows the shell's call events", async ({ page }) => {
  await openRecord(page);
  await mobile(page, { type: "callActive", active: true });
  await page.getByRole("button", { name: "Record room" }).click();
  await expect(page.getByRole("dialog", { name: "Phones can’t record calls" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await mobile(page, { type: "callActive", active: false });
  await page.getByRole("button", { name: "Record room" }).click();
  await expect(page.getByRole("dialog", { name: "Tell everyone you’re recording" })).toBeVisible();
});

test("a start refused while the voice enrollment holds the microphone", async ({ page }) => {
  await openRecord(page, { knobs: { failStart: "micInUse" } });
  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText("The microphone is in use. Finish your voice recording first.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Record room" })).toBeVisible();
  await page.getByRole("button", { name: "Dismiss" }).click();
  await expect(page.getByText(/The microphone is in use/)).toHaveCount(0);
});

for (const [code, text] of [
  ["diskLow", "Not enough storage to record. Free up space and try again."],
  ["pairingNotAvailable", "Sending to a computer isn’t available yet. Choose This phone."],
  ["waitingForTranscription", "Finishing the last recording… try again in a moment."],
] as const) {
  test(`start refused: ${code}`, async ({ page }) => {
    await openRecord(page, { knobs: { failStart: code } });
    await page.getByRole("button", { name: "Record room" }).click();
    await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
    await expect(page.getByText(text)).toBeVisible();
    await expectAccessible(page);
  });
}

test("start refused: a call began since the screen looked reopens the call notice", async ({ page }) => {
  await openRecord(page);
  await page.getByRole("button", { name: "Record room" }).click();
  await setKnobs(page, { callActive: true });
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByRole("dialog", { name: "Phones can’t record calls" })).toBeVisible();
  await page.getByRole("button", { name: "Use speakerphone and Room mode" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
});

test("start refused: microphone denied turns the control into the Fix button", async ({ page }) => {
  await openRecord(page);
  await page.getByRole("button", { name: "Record room" }).click();
  await setKnobs(page, { mic: "denied" });
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText("Microphone unavailable")).toBeVisible();
});

for (const action of ["Pause", "Mark", "Stop"]) {
  test(`${action} failing says so`, async ({ page }) => {
    await openRecord(page);
    await live(page);
    await setKnobs(page, { failAction: true });
    await page.getByRole("button", { name: action, exact: action !== "Mark" }).first().click();
    await expect(page.getByText("That didn’t work. Try again.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
    await page.getByRole("button", { name: "Dismiss" }).click();
    await expect(page.getByText("That didn’t work.")).toHaveCount(0);
  });
}

test("the banner region exists only when there is something to say", async ({ page }) => {
  await openRecord(page);
  await expect(page.getByRole("region", { name: "Room recording" })).toHaveCount(0);
  await live(page);
  await expect(page.getByRole("region", { name: "Room recording" })).toHaveCount(0);
  await phase(page, "locked");
  await expect(page.getByRole("region", { name: "Room recording" })).toBeVisible();
});

test("any other start error is said plainly", async ({ page }) => {
  await openRecord(page, { knobs: { failStart: "disk" } });
  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText("Couldn’t start recording. Try again.")).toBeVisible();
});

test("microphone denied: the control says so and opens Settings", async ({ page }) => {
  await openRecord(page, { knobs: { mic: "denied" } });
  await expect(page.getByText("Microphone unavailable")).toBeVisible();
  await page.getByRole("button", { name: "Fix" }).click();
  expect(await log(page)).toContain("openAppSettings");
  await expectAccessible(page);
});

test("a reloaded webview picks the session up where it is", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 5);
  await page.getByRole("button", { name: "Mark" }).click();
  await expect(lines(page)).toHaveCount(5);
  await page.reload();
  await page.waitForFunction(() => Boolean(window.__ghiMock));
  await expect(page.locator("[data-screen=record]")).toBeVisible();
  await expect(lines(page)).toHaveCount(5);
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark, 1 mark" })).toBeVisible();
  await expect(lines(page).first().locator("[data-icon=star]")).toHaveCount(1);
  // New lines continue from there, without repeats.
  await addLines(page, 2);
  await expect(lines(page)).toHaveCount(7);
});

test("a reloaded catching-up session shows its banner", async ({ page }) => {
  await openRecord(page);
  await page.evaluate(() => {
    window.__ghiRecord!.seed("catchingUp", 5);
    window.location.hash = "#/meetings";
  });
  await page.evaluate(() => (window.location.hash = "#/record"));
  await expect(page.getByText(/Catching up/)).toBeVisible();
  await expect(lines(page)).toHaveCount(5);
});

test("waking the app refreshes without doubling lines", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 3);
  await expect(lines(page)).toHaveCount(3);
  // The snapshot is read while more lines arrive.
  await page.evaluate(() => {
    document.dispatchEvent(new Event("visibilitychange"));
    window.__ghiRecord!.addLines(2);
  });
  await expect(lines(page)).toHaveCount(5);
  await page.waitForTimeout(200);
  await expect(lines(page)).toHaveCount(5);
});

test("an interrupted session found after a reload asks neutrally until it knows", async ({ page }) => {
  await openRecord(page);
  await page.evaluate(() => {
    window.__ghiRecord!.seed("interrupted", 2);
    window.location.hash = "#/meetings";
  });
  await page.evaluate(() => (window.location.hash = "#/record"));
  await expect(page.getByRole("dialog", { name: "Recording paused" })).toBeVisible();
});

test("a long meeting stays smooth: 1,500 lines, only a window of them in the DOM", async ({ page }) => {
  await openRecord(page);
  await live(page);
  const took = await page.evaluate(async () => {
    const t0 = performance.now();
    window.__ghiRecord!.addLines(1500);
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    return performance.now() - t0;
  });
  expect(took).toBeLessThan(4000);
  // Following the newest line.
  await expect(page.getByTestId("line").last()).toContainText(/Monday|Friday|bản beta|quý bốn|ngân sách|kiểm tra|ỗ/);
  const rendered = await lines(page).count();
  expect(rendered).toBeGreaterThan(3);
  expect(rendered).toBeLessThan(60);

  // Scrolling up stops following and offers the way back.
  const log_ = page.getByRole("log", { name: "Live transcript" });
  // A finger drag up (mobile WebKit has no wheel in tests).
  await log_.evaluate((el) => {
    el.dispatchEvent(new Event("touchmove", { bubbles: true }));
    el.scrollTop = 0;
  });
  await expect(page.getByRole("button", { name: "Latest" })).toBeVisible();
  await page.getByRole("button", { name: "Latest" }).click();
  await expect(page.getByRole("button", { name: "Latest" })).toHaveCount(0);

  // Sustained input stays cheap: the next 200 lines in one frame budget.
  const more = await page.evaluate(async () => {
    const t0 = performance.now();
    window.__ghiRecord!.addLines(200);
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    return performance.now() - t0;
  });
  expect(more).toBeLessThan(2000);
});

test("sustained levels and partials stay cheap: 20 s of 10 levels/s, 3 partials/s, a line every 4 s", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 300);
  const frames = await page.evaluate(async () => {
    const mock = window.__ghiMock!;
    const times: number[] = [];
    let last = performance.now();
    // 200 frames of 100 ms of meeting time each.
    for (let i = 0; i < 200; i += 1) {
      mock.simulateCoreEvent({ seq: null, atMs: null, event: { type: "levelMeter", meeting: "m-1", micDbfs: -40 + (i % 30), systemDbfs: null } });
      if (i % 3 === 0) mock.simulateCoreEvent({ seq: null, atMs: null, event: { type: "transcriptPartial", meeting: "m-1", track: 0, text: `chốt scope ${i}` } });
      if (i % 40 === 0) window.__ghiRecord!.addLines(1);
      await new Promise((r) => requestAnimationFrame(r));
      const now = performance.now();
      times.push(now - last);
      last = now;
    }
    return times;
  });
  const mean = frames.reduce((a, b) => a + b, 0) / frames.length;
  expect(mean).toBeLessThan(50);
  expect(Math.max(...frames)).toBeLessThan(500);
  await expect(lines(page).last()).toBeVisible();
});

test("reduced motion: the waveform holds still", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openRecord(page);
  await live(page);
  await expect(page.getByTestId("waveform")).toHaveAttribute("data-moving", "false");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(page.getByTestId("waveform")).toHaveAttribute("data-moving", "true");
});

test("speakers are color + initial, never color alone", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 3);
  for (const [i, n] of ["1", "2", "3"].entries()) {
    const row = lines(page).nth(i);
    await expect(row).toContainText(`Speaker ${n}`);
    await expect(row.locator("[data-kind], [class*=rounded-full]").first()).toContainText(n);
  }
});

test("Vietnamese copy", async ({ page }) => {
  await openRecord(page, { lang: "vi" });
  await expect(page.getByRole("heading", { name: "Ghi âm phòng họp" })).toBeVisible();
  await page.getByRole("button", { name: "Ghi phòng họp" }).click();
  await expect(page.getByRole("dialog", { name: "Hãy báo cho mọi người biết bạn đang ghi âm" })).toBeVisible();
});

test.describe("axe", () => {
  for (const lang of ["en", "vi"] as const) {
    test(`idle, consent sheet, live (${lang})`, async ({ page }) => {
      await openRecord(page, { lang });
      await expectAccessible(page);
      await page.getByRole("button", { name: /record room|ghi phòng họp/i }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await expectAccessible(page);
      await page.getByRole("button", { name: /start recording|bắt đầu ghi âm/i }).click();
      await expect(page.getByRole("button", { name: /^(Stop|Dừng)$/ })).toBeVisible();
      await addLines(page, 5);
      await expectAccessible(page);
    });

    test(`interruption sheet (${lang})`, async ({ page }) => {
      await openRecord(page, { lang });
      await startRecording(page, /record room|ghi phòng họp/i);
      // Stop shows while loading; the mock goes live a moment later and would overwrite the phase below.
      await expect(page.getByText(/Getting ready|Đang chuẩn bị/)).toHaveCount(0);
      await mobile(page, { type: "interruption", began: true, kind: "call" });
      await phase(page, "interrupted");
      await expect(page.getByRole("dialog")).toBeVisible();
      await expectAccessible(page);
    });
  }
});
