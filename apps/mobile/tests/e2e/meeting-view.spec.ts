// SPDX-License-Identifier: Apache-2.0
// M4: the meeting view on the scripted mock.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import {
  mock,
  openMeetings,
  recorded,
  recordPlatform,
} from "./meetings-helpers";

test.describe("meeting view", () => {
  test.beforeEach(async ({ page }) => {
    await recordPlatform(page);
  });

  test("tabs: notes with provenance, actions, transcript", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    await expect(
      page.getByRole("heading", { level: 1, name: "Product sync tuần 39" }),
    ).toBeVisible();
    await expect(page.locator('[data-chip="synced"]')).toBeVisible();
    await expect(
      page.getByRole("status").filter({ hasText: "Local only" }),
    ).toBeVisible();

    await expect(
      page.getByRole("tab", { name: "Notes", selected: true }),
    ).toBeVisible();
    await expect(page.getByRole("heading", { name: "Summary" })).toBeVisible();
    await expect(
      page.getByText("The team will ship the beta on 15 October"),
    ).toBeVisible();
    await expect(page.getByText("You wrote").first()).toBeVisible();
    await expect(
      page.getByText(/^Written by .* from the transcript$/).first(),
    ).toBeVisible();
    await expect(page.getByText(/^Edited by you/)).toBeVisible();
    await expect(page.getByText(/^Not found in the transcript/)).toBeVisible();

    await page.getByRole("tab", { name: "Actions" }).click();
    await expect(
      page.getByText("Send the revised budget before the review"),
    ).toBeVisible();
    const done = page.getByRole("checkbox", {
      name: "Send the revised budget before the review",
    });
    await expect(done).not.toBeChecked();
    await done.click();
    await expect(done).toBeChecked();
    // The mock kept it: back to the list and in again.
    await page.getByRole("tab", { name: "Notes" }).click();
    await page.getByRole("tab", { name: "Actions" }).click();
    await expect(
      page.getByRole("checkbox", {
        name: "Send the revised budget before the review",
      }),
    ).toBeChecked();

    await page.getByRole("tab", { name: "Transcript" }).click();
    // Three lines around 1:30, six more across the half hour (the waveform's colours).
    await expect(page.locator("[data-segment]")).toHaveCount(9);
    await expect(page.getByText("Chốt scope cho bản beta")).toBeVisible();
  });

  test("a citation opens the quote sheet and plays from its time", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-notes");
    await page
      .getByRole("button", { name: "Show in transcript 01:30" })
      .click();
    const sheet = page.getByRole("dialog", { name: "From the transcript" });
    await expect(sheet).toContainText(
      "We can ship the beta on the fifteenth if QA signs off by Friday.",
    );
    await expect(sheet).toContainText("Linh");
    await expect(sheet).toContainText("01:30");
    await sheet.getByRole("button", { name: "Play from 01:30" }).click();
    await expect(sheet).toHaveCount(0);
    await expect
      .poll(async () => (await recorded(page)).audio.at(-1))
      .toEqual({ op: "play", t: 90 });
    // The chip remembers it was followed.
    await expect(
      page.getByRole("button", { name: "Show in transcript 01:30" }),
    ).toHaveAttribute("data-state", "visited");
  });

  test("a citation with no words says so and has nothing to play", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-notes");
    await page
      .getByRole("button", { name: /Show in transcript 03:20/ })
      .click();
    const sheet = page.getByRole("dialog", { name: "From the transcript" });
    await expect(sheet).toContainText(
      "No matching words were found at that moment.",
    );
    await expect(sheet.getByRole("button", { name: /Play from/ })).toHaveCount(
      0,
    );
  });

  test("an action's citation plays too", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes?tab=actions");
    await page
      .getByRole("button", { name: "Show in transcript 01:45" })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Play from 01:45" })
      .click();
    await expect
      .poll(async () => (await recorded(page)).audio.at(-1))
      .toEqual({ op: "play", t: 105 });
  });

  test("the audio bar plays, pauses and changes speed", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    await page.getByRole("button", { name: "Play", exact: true }).click();
    await expect(page.getByRole("button", { name: "Pause" })).toBeVisible();
    await page.getByRole("button", { name: "Pause" }).click();
    await expect
      .poll(async () => (await recorded(page)).audio.map((a) => a.op))
      .toEqual(["play", "pause"]);
    await expect(
      page.getByRole("button", { name: "Playback speed 1×" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Playback speed 1×" }).click();
    await expect(
      page.getByRole("button", { name: "Playback speed 1.25×" }),
    ).toBeVisible();
    await expect(
      page.getByRole("slider", { name: "Position" }),
    ).toHaveAttribute("aria-valuetext", "0:00 / 31:00");
  });

  test("tapping and dragging the waveform seeks", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    const seek = page.getByTestId("audio-seek");
    const box = (await seek.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(44);
    const slider = page.getByRole("slider", { name: "Position" });
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect(slider).toHaveAttribute("aria-valuetext", /^15:/);
    // Drag to the start: it scrubs.
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + 2, box.y + box.height / 2, { steps: 5 });
    await page.mouse.up();
    await expect(slider).toHaveAttribute("aria-valuetext", /^0:\d\d \/ 31:00$/);
  });

  test("a transcript line edit persists", async ({ page }) => {
    await openMeetings(page, "/meetings/m-nonotes?tab=transcript");
    const line = page.locator('[data-segment="s2"]');
    await line.getByText("Ngân sách dự kiến").click();
    await line.getByRole("button", { name: "Play from 00:12" }).click();
    await expect
      .poll(async () => (await recorded(page)).audio.at(-1))
      .toEqual({ op: "play", t: 12.5 });
    await line.getByRole("button", { name: "Edit line" }).click();
    const box = line.getByRole("textbox", { name: "Transcript text" });
    await box.fill("Ngân sách dự kiến là sáu trăm triệu đồng.");
    await line.getByRole("button", { name: "Save" }).click();
    await expect(line).toContainText("sáu trăm triệu đồng");
    await expect(line).toContainText("Edited");
    // Leave and come back: the mock core kept it.
    await page.getByRole("button", { name: /Back/ }).click();
    await page
      .locator('[data-meeting="m-nonotes"]')
      .getByRole("button", { name: /^Họp kế hoạch/ })
      .click();
    await page.getByRole("tab", { name: "Transcript" }).click();
    await expect(page.locator('[data-segment="s2"]')).toContainText(
      "sáu trăm triệu đồng",
    );
  });

  test("cancel leaves the line as it was", async ({ page }) => {
    await openMeetings(page, "/meetings/m-nonotes?tab=transcript");
    const line = page.locator('[data-segment="s1"]');
    await line.getByText("Chào mọi người").click();
    await line.getByRole("button", { name: "Edit line" }).click();
    await line.getByRole("textbox").fill("khác");
    await line.getByRole("button", { name: "Cancel" }).click();
    await expect(line).toContainText("Chào mọi người");
  });

  test("a meeting without notes explains and opens the cloud sheet", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-nonotes");
    // The cloud entry exists once cloud notes are offered in Settings.
    await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(true));
    await page.evaluate(() => (location.hash = "#/meetings"));
    await page.evaluate(() => (location.hash = "#/meetings/m-nonotes"));
    await expect(
      page.getByRole("heading", { name: "Notes: not generated on this phone" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Improve with cloud" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("shares Markdown and plain text through the native share sheet", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-notes");
    const shared = () =>
      page.evaluate(
        () => (window as unknown as { __ghiShared: unknown[] }).__ghiShared,
      );
    await page.getByRole("button", { name: "Share" }).click();
    await page.getByRole("button", { name: "Markdown (.md)" }).click();
    await expect.poll(shared).toEqual([{ meeting: "m-notes", format: "md" }]);
    await page.getByRole("button", { name: "Share" }).click();
    await page.getByRole("button", { name: "Plain text (.txt)" }).click();
    await expect.poll(shared).toEqual([
      { meeting: "m-notes", format: "md" },
      { meeting: "m-notes", format: "txt" },
    ]);
  });

  test("a meeting that is gone says so", async ({ page }) => {
    await openMeetings(page, "/meetings/nope");
    await expect(
      page.getByText("This meeting isn’t on this phone any more."),
    ).toBeVisible();
  });

  test("a meeting opens once the app unlocks", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes", { locked: true });
    await expect(
      page.getByRole("heading", { level: 1, name: "Product sync tuần 39" }),
    ).toBeVisible();
  });

  test("a meeting opens once the core has started", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes", { starting: 800 });
    await expect(
      page.getByRole("heading", { level: 1, name: "Product sync tuần 39" }),
    ).toBeVisible();
  });

  test("the list loads once the app unlocks", async ({ page }) => {
    await openMeetings(page, "/meetings", { locked: true });
    await expect(page.locator('[data-meeting="m-notes"]')).toBeVisible();
  });

  test("locking closes the quote and share sheets and a pending delete", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-notes");
    const lock = () =>
      page.evaluate(() => window.dispatchEvent(new Event("ghi-locked")));
    await page
      .getByRole("button", { name: "Show in transcript 01:30" })
      .click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await lock();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "Share" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await lock();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await openMeetings(page, "/meetings");
    const row = page.locator('[data-meeting="m-old2"]');
    await row.getByRole("button", { name: /^Delete/ }).focus();
    await row.getByRole("button", { name: /^Delete/ }).click();
    await expect(page.getByRole("alertdialog")).toBeVisible();
    await lock();
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
  });

  test("a failed share offers an explicit Copy, never copies on its own", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-notes");
    await mock(page, "failShare", true);
    await page.getByRole("button", { name: "Share" }).click();
    await page.getByRole("button", { name: "Markdown (.md)" }).click();
    await expect(page.getByRole("alert")).toContainText(
      "Couldn’t open the share sheet",
    );
    expect((await recorded(page)).copied).toEqual([]);
    await page.getByRole("button", { name: "Copy to clipboard" }).click();
    await expect(page.getByText("Copied to clipboard")).toBeVisible();
    const [text] = (await recorded(page)).copied;
    expect(text).toContain("# Product sync tuần 39");
  });

  test("Never send to cloud is a switch that sticks", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    const sw = page.getByRole("switch", { name: "Never send to cloud" });
    await expect(sw).not.toBeChecked();
    await sw.click();
    await expect(sw).toBeChecked();
    await page.getByRole("button", { name: /Back/ }).click();
    await page
      .locator('[data-meeting="m-notes"]')
      .getByRole("button", { name: /^Product sync/ })
      .click();
    await expect(
      page.getByRole("switch", { name: "Never send to cloud" }),
    ).toBeChecked();
  });

  test("audio that cannot start can be tried again", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    await mock(page, "failAudio", true);
    await page.getByRole("button", { name: "Play", exact: true }).click();
    await expect(
      page.getByText("Audio isn’t available for this meeting."),
    ).toBeVisible();
    await mock(page, "failAudio", false);
    await page.getByRole("button", { name: "Play", exact: true }).click();
    await expect(page.getByRole("button", { name: "Pause" })).toBeVisible();
    await expect(
      page.getByText("Audio isn’t available for this meeting."),
    ).toHaveCount(0);
  });

  test("a failed save keeps the editor open with a message", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-nonotes?tab=transcript");
    await mock(page, "failSaves", true);
    const line = page.locator('[data-segment="s1"]');
    await line.getByText("Chào mọi người").click();
    await line.getByRole("button", { name: "Edit line" }).click();
    await line.getByRole("textbox").fill("khác");
    await line.getByRole("button", { name: "Save" }).click();
    await expect(line.getByRole("alert")).toContainText(
      "Couldn’t save this line",
    );
    await expect(line.getByRole("textbox")).toHaveValue("khác");
  });

  test("a transcript line is one real control, reachable by keyboard", async ({
    page,
  }) => {
    await openMeetings(page, "/meetings/m-nonotes?tab=transcript");
    const line = page
      .locator('[data-segment="s1"]')
      .getByRole("button", { name: /Chào mọi người/ });
    await line.focus();
    await page.keyboard.press("Enter");
    await expect(line).toHaveAttribute("aria-expanded", "true");
    await expect(
      page
        .locator('[data-segment="s1"]')
        .getByRole("button", { name: "Edit line" }),
    ).toBeVisible();
  });

  test("only the selected tab points at a panel", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    await expect(page.getByRole("tab", { name: "Notes" })).toHaveAttribute(
      "aria-controls",
      "panel-notes",
    );
    await expect(
      page.getByRole("tab", { name: "Actions" }),
    ).not.toHaveAttribute("aria-controls", /.+/);
  });

  test("is accessible on every tab, EN and VI at 200%", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    for (const tab of ["Notes", "Actions", "Transcript"]) {
      await page.getByRole("tab", { name: tab }).click();
      await expect(
        page.getByRole("tab", { name: tab, selected: true }),
      ).toBeVisible();
      await expectAccessible(page);
    }
    await openMeetings(page, "/meetings/m-notes", { lang: "vi", scale: 2 });
    await expect(
      page.getByRole("tab", { name: "Ghi chú", selected: true }),
    ).toBeVisible();
    await expectAccessible(page);
    await page.getByRole("button", { name: "Chia sẻ" }).click();
    await expectAccessible(page);
  });
});

test.describe("sensitive meeting", () => {
  test.beforeEach(async ({ page }) => {
    await recordPlatform(page);
  });

  test("turning it on asks first, deletes the audio, hides playback and shows the badge", async ({ page }) => {
    await openMeetings(page, "/meetings/m-notes");
    await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);
    await expect(page.getByRole("group", { name: "Audio player" })).toBeVisible();

    const sw = page.getByRole("switch", { name: "Sensitive meeting" });
    await expect(sw).not.toBeChecked();
    await sw.click();
    const sheet = page.getByRole("dialog", { name: "Make this meeting sensitive?" });
    await expect(sheet).toContainText("Its audio is deleted now and only the transcript stays");
    await sheet.getByRole("button", { name: "Cancel" }).click();
    await expect(sw).not.toBeChecked();
    await expect(page.getByRole("group", { name: "Audio player" })).toBeVisible();

    await sw.click();
    await page.getByRole("dialog", { name: "Make this meeting sensitive?" }).getByRole("button", { name: "Make sensitive" }).click();
    await expect(sw).toBeChecked();
    await expect(page.getByTestId("sensitive-badge")).toHaveText("Sensitive · no audio kept");
    await expect(page.getByRole("group", { name: "Audio player" })).toHaveCount(0);
    // No cloud for it: the never-send switch is on and can't be changed.
    await expect(page.getByRole("switch", { name: "Never send to cloud" })).toBeChecked();
    await expect(page.getByRole("switch", { name: "Never send to cloud" })).toBeDisabled();

    // No "Play from" on a line either.
    await page.getByRole("tab", { name: "Transcript" }).click();
    await page.locator("[data-segment]").first().click();
    await expect(page.getByRole("button", { name: /Play from/ })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Edit" })).toBeVisible();
    await expectAccessible(page);

    // Off again: the flag only; the audio stays deleted.
    await page.getByRole("tab", { name: "Notes" }).click();
    await sw.click();
    await expect(sw).not.toBeChecked();
    await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);
    await expect(page.getByRole("group", { name: "Audio player" })).toHaveCount(0);
  });
});
