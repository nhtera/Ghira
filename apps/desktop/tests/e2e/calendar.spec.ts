// SPDX-License-Identifier: Apache-2.0
// Calendar (phase 14d) on the mocked core: the Up next strip and its switch,
// the popover Next row, Settings → Recording, the onboarding row, and the two
// kinds of detection prompt. The mock's flags: ?calendar=1 (access granted),
// =denied, =ics (a file is connected); with none, macOS has not been asked.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, hash: string, calendar?: string, platform = "mac") =>
  page.goto(
    `/?platform=${platform}${calendar ? `&calendar=${calendar}` : ""}#${hash}`,
  );
const card = (page: Page) =>
  page
    .locator("section")
    .filter({ has: page.getByRole("heading", { level: 3, name: "Calendar" }) });
const detect = (page: Page, e: object) =>
  page.evaluate(
    (d) =>
      (
        window as unknown as {
          __ghiMock: { simulateMeetingDetected: (d: object) => void };
        }
      ).__ghiMock.simulateMeetingDetected(d),
    e,
  );

test("Up next shows the next meeting-like event; its switch is kept", async ({
  page,
}) => {
  await open(page, "/meetings", "1");
  const strip = page.getByRole("region", { name: "Up next" });
  await expect(strip).toContainText("Sprint planning");
  await expect(strip).toContainText("Zoom");
  // Lunch has nobody else and no call link: never the next meeting.
  await expect(strip).not.toContainText("Lunch");
  const sw = strip.getByRole("switch", {
    name: "Ask to record when it starts",
  });
  await expect(sw).toHaveAttribute("aria-checked", "true");
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "false");
});

test("no strip without a calendar", async ({ page }) => {
  await open(page, "/meetings");
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Up next" })).toHaveCount(0);
});

test("an .ics file feeds the strip too", async ({ page }) => {
  await open(page, "/meetings", "ics", "win");
  await expect(page.getByRole("region", { name: "Up next" })).toContainText(
    "Sprint planning",
  );
});

test("popover: the Next row has the same switch", async ({ page }) => {
  await open(page, "/popover", "1");
  const next = page.getByRole("region", { name: "Next" });
  await expect(next).toContainText("Sprint planning");
  await expect(
    next.getByRole("switch", { name: "Ask to record when it starts" }),
  ).toHaveAttribute("aria-checked", "true");
});

test("settings: connect Calendar, then the ask switch appears", async ({
  page,
}) => {
  await open(page, "/settings/recording");
  const c = card(page);
  await expect(c).toContainText("Calendar data stays on this Mac");
  await expect(c.getByRole("switch")).toHaveCount(0);
  await c.getByRole("button", { name: "Connect Calendar…" }).click();
  await expect(c.getByText("Connected")).toBeVisible();
  await expect(
    c.getByRole("switch", {
      name: "Ask to record when a calendar meeting starts",
    }),
  ).toHaveAttribute("aria-checked", "true");
});

test("settings: denied explains where to turn it on", async ({ page }) => {
  await open(page, "/settings/recording", "denied");
  const c = card(page);
  await expect(c).toContainText("Calendar access is off for");
  await expect(
    c.getByRole("button", { name: "Open System Settings" }),
  ).toBeVisible();
});

test("settings: import and remove an .ics file (Windows)", async ({ page }) => {
  await open(page, "/settings/recording", undefined, "win");
  const c = card(page);
  await expect(c).toContainText("Calendar data stays on this PC");
  await c
    .getByRole("button", { name: "Import a calendar file (.ics)…" })
    .click();
  await expect(c.getByText("From work.ics")).toBeVisible();
  await expect(
    c.getByRole("switch", {
      name: "Ask to record when a calendar meeting starts",
    }),
  ).toBeVisible();
  await c.getByRole("button", { name: "Remove calendar file" }).click();
  await expect(c.getByText("From work.ics")).toHaveCount(0);
});

test("onboarding: Allow… asks for Calendar on macOS, and is not there on Windows", async ({
  page,
}) => {
  await open(page, "/onboarding/permissions");
  const row = page
    .getByRole("listitem")
    .filter({ hasText: "Calendar · optional" });
  await row.getByRole("button", { name: /^Allow Calendar/ }).click();
  await expect(row.getByText("Allowed")).toBeVisible();
  await open(page, "/onboarding/permissions", undefined, "win");
  await expect(
    page.getByRole("heading", { name: /to hear your meetings/ }),
  ).toBeVisible();
  await expect(page.getByText("Calendar · optional")).toHaveCount(0);
});

test("a call inside a calendar event carries its title", async ({ page }) => {
  await open(page, "/meetings", "1");
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await detect(page, {
    app: "zoom",
    appName: "Zoom",
    browser: false,
    title: "Sprint planning",
    event: "ev-1@1",
  });
  const prompt = page.getByRole("region", {
    name: "Zoom call detected. Record it?",
  });
  await expect(prompt).toContainText("Sprint planning · from your calendar");
  await expect(
    prompt.getByRole("button", { name: "Never for Zoom" }),
  ).toBeVisible();
});

test("a calendar start offers Start and Not now, never Never", async ({
  page,
}) => {
  await open(page, "/meetings", "1");
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await detect(page, {
    app: "calendar",
    appName: "",
    browser: false,
    title: "Sprint planning",
    event: "ev-1@1",
  });
  const prompt = page.getByRole("region", {
    name: "“Sprint planning” is starting. Record it?",
  });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("button", { name: /Never/ })).toHaveCount(0);
  await prompt.getByRole("button", { name: "Not now" }).click();
  await expect(prompt).toHaveCount(0);
});

test("detect panel for a calendar start", async ({ page }) => {
  await open(
    page,
    "/detect?app=calendar&name=&browser=0&title=Sprint%20planning&event=ev-1%401",
  );
  const prompt = page.getByRole("region", {
    name: "“Sprint planning” is starting. Record it?",
  });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("button", { name: "Start" })).toBeVisible();
  await expect(prompt.getByRole("button", { name: /Never/ })).toHaveCount(0);
});
