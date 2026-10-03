// SPDX-License-Identifier: Apache-2.0
// Import (D10) on the mocked core: choose files, problems inline, queue, done.
import { expect, test } from "@playwright/test";

test("choose files → problems shown → import → queue → done with Open notes", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  await expect(page.getByText("Drop audio or video files here")).toBeVisible();
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  await expect(staged).toBeVisible();
  // The sample list has a duplicate, a very long file and an unreadable one.
  await expect(staged.getByText(/Already imported as “Client call — Acme onboarding”/)).toBeVisible();
  await expect(staged.getByRole("button", { name: "Open", exact: true })).toBeVisible();
  await expect(staged.getByText(/Over 4 hours/)).toBeVisible();
  await expect(staged.getByText(/can’t read this file/)).toBeVisible();
  // A stereo file offers split channels.
  await expect(page.getByRole("checkbox", { name: /Split stereo channels/ })).toBeVisible();
  // Remove the blocked ones, then import the rest.
  await staged.getByRole("button", { name: /^Remove / }).last().click();
  const go = page.getByRole("button", { name: /^Import \d+ files?$/ });
  await expect(go).toBeEnabled();
  await go.click();
  const queue = page.getByRole("region", { name: "Queue" });
  await expect(queue.getByText(/files? importing/)).toBeVisible();
  await expect(queue.getByText(/Converting \d+%/).first()).toBeVisible({ timeout: 8000 });
  await expect(queue.getByRole("button", { name: "Open notes" }).first()).toBeVisible({ timeout: 20000 });
  await expect(page.getByText(/^\d+ files? imported\. Notes are ready\.$/)).toBeVisible({ timeout: 20000 });
});

test("files dragged over the window show the drop target", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  // The screen listens once it has mounted.
  await expect(page.getByRole("button", { name: "Choose files…" })).toBeVisible();
  await page.evaluate(() => {
    const dt = new DataTransfer();
    dt.items.add(new File(["x"], "a.mp3"));
    window.dispatchEvent(new DragEvent("dragenter", { dataTransfer: dt }));
  });
  await expect(page.getByText("Drop to import into Ghira")).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new DragEvent("dragleave", { dataTransfer: new DataTransfer() })));
});

test("a drop staged by the core appears in the list", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  // The mock installs its hooks once the app has started.
  await page.waitForFunction(() => "__ghiMock" in window);
  await page.evaluate(() => (window as unknown as { __ghiMock: { simulateImportDrop: () => void } }).__ghiMock.simulateImportDrop());
  await expect(page.getByRole("region", { name: "Ready to import" })).toBeVisible();
});

test("a Zoom recording's participant tracks import as one meeting", async ({ page }) => {
  await page.goto("/?platform=win&importgroup=1#/import");
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  // One row for the recording, with its participants (one has no name in the file).
  await expect(staged.getByText("Zoom recording · 4 participants")).toBeVisible();
  for (const name of ["Linh", "Minh", "Sarah", "Participant 4"]) await expect(staged.getByText(name, { exact: true })).toBeVisible();
  // The mixed recording of the same folder is not imported.
  await expect(staged.getByText("audio_only.m4a")).toBeVisible();
  await expect(staged.getByText("Won’t be imported")).toBeVisible();
  // Removing a participant leaves the rest as the recording.
  await staged.getByRole("button", { name: "Remove Sarah" }).click();
  await expect(staged.getByText("Zoom recording · 3 participants")).toBeVisible();
  const go = page.getByRole("button", { name: "Import 3 files" });
  await go.click();
  // One queue item, named for the meeting.
  const queue = page.getByRole("region", { name: "Queue" });
  await expect(queue.getByText("Sprint planning")).toBeVisible();
  await expect(queue.getByText(/1 file importing/)).toBeVisible();
  await expect(queue.getByRole("button", { name: "Open notes" })).toBeVisible({ timeout: 20000 });
});

test("a Zoom recording imported before is blocked as a whole", async ({ page }) => {
  await page.goto("/?platform=win&importgroup=dup#/import");
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  await expect(staged.getByText(/Already imported as “Sprint planning”/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Import 0 files" })).toBeDisabled();
});

test("a Zoom recording can be imported track by track, and the mixed recording comes back", async ({ page }) => {
  await page.goto("/?platform=win&importgroup=1#/import");
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  await expect(staged.getByText("The Zoom participant tracks are imported instead.")).toBeVisible();
  await staged.getByRole("button", { name: "Import tracks separately" }).click();
  // Four files on their own, and the mixed recording is no longer left out.
  await expect(staged.getByText(/Zoom recording · \d+ participants/)).toHaveCount(0);
  await expect(staged.getByText("The Zoom participant tracks are imported instead.")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Import 5 files" })).toBeEnabled();
});

test("removing every track brings the mixed Zoom recording back", async ({ page }) => {
  await page.goto("/?platform=win&importgroup=1#/import");
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  for (const name of ["Linh", "Minh", "Sarah"]) await staged.getByRole("button", { name: `Remove ${name}` }).click();
  await staged.getByRole("button", { name: "Remove audio_recording_4.m4a" }).click();
  await expect(staged.getByText("audio_only.m4a")).toBeVisible();
  await expect(staged.getByText("Won’t be imported")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Import 1 file" })).toBeEnabled();
});
