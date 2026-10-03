// SPDX-License-Identifier: Apache-2.0
// "Improve with cloud…" send sheet on the mocked core (copy matched as English).
import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page, query = "platform=win") => {
  await page.goto(`/?${query}#/meetings`);
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
};
const addKey = (page: Page, provider = "openai") =>
  page.evaluate((p) => (window as unknown as { __ghiMock: { setCloudKey: (p: string) => unknown } }).__ghiMock.setCloudKey(p), provider);

test("without a key the sheet links to Settings and cannot send", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Improve with cloud…" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByText(/No API key for/)).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Send and improve" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Add a key in Settings → AI" }).click();
  await expect(page).toHaveURL(/#\/settings\/ai/);
});

test("the preview shows the exact payload and re-previews when redaction changes", async ({ page }) => {
  await open(page);
  await addKey(page);
  await page.getByRole("button", { name: "Improve with cloud…" }).click();
  const sheet = page.getByRole("dialog");
  // The exact request is one click away, closed until asked for.
  await sheet.getByText("Show exact data").click();
  const payload = sheet.getByTestId("cloud-payload");
  await expect(payload).toContainText("messages");
  await expect(sheet.getByText("Sent to api.openai.com")).toBeVisible();
  await expect(sheet.getByText("Audio never leaves this device")).toBeVisible();
  const sha = await payload.getAttribute("data-sha");
  // The switch is a visually hidden checkbox; people click its label.
  const hide = sheet.getByRole("switch", { name: "Hide names and personal data" });
  await expect(hide).toBeChecked();
  await sheet.getByText("Hide names and personal data", { exact: true }).click();
  await expect(hide).not.toBeChecked();
  await expect(payload).not.toHaveAttribute("data-sha", sha!);
});

test("sending improves the notes: toast, amber cloud chip", async ({ page }) => {
  await open(page);
  await addKey(page);
  await page.getByRole("button", { name: "Improve with cloud…" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByTestId("cloud-excerpt")).toBeVisible();
  await sheet.getByRole("button", { name: "Send and improve" }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByText(/Notes improved/).first()).toBeVisible();
  await expect(page.getByTestId("meeting-detail").getByText("Cloud-enhanced").first()).toBeVisible();
});

test("a failed send says the notes are written on this device instead", async ({ page }) => {
  await open(page, "platform=win&cloudfail=1");
  await addKey(page);
  await page.getByRole("button", { name: "Improve with cloud…" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByTestId("cloud-excerpt")).toBeVisible();
  await sheet.getByRole("button", { name: "Send and improve" }).click();
  await expect(sheet.getByText("Writing them on this device instead.")).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Send and improve" })).toHaveCount(0);
});

test("Never send to cloud turns the sheet into an explanation", async ({ page }) => {
  await open(page);
  await addKey(page);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Never send to cloud" }).click();
  await page.getByRole("button", { name: "Improve with cloud…" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByText("Cloud AI is turned off for this meeting.")).toBeVisible();
  await expect(sheet.getByRole("button", { name: "Send and improve" })).toBeDisabled();
});
