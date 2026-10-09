// SPDX-License-Identifier: Apache-2.0
// Your own note templates on the mocked core: create in Settings, regenerate a
// meeting with it, rename and delete it, and the notes keep every section.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

// The mock core plays a blob: WAV, which the production media-src refuses.
test.use({ bypassCSP: true });

const toSettings = async (page: Page) => {
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await page.getByRole("link", { name: "Templates", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Templates", level: 2 })).toBeVisible();
};
const toMeeting = async (page: Page) => {
  await page.getByRole("link", { name: "Meetings", exact: true }).click();
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
};

test("create → regenerate with it → rename → delete: the sections stay in the notes", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await toSettings(page);

  // Create.
  await page.getByRole("button", { name: "New template" }).click();
  const editor = page.getByTestId("template-editor");
  await editor.getByRole("textbox", { name: "Name" }).fill("Weekly retro");
  await editor.getByRole("textbox", { name: "Section 1: title" }).fill("Went well");
  await editor.getByRole("textbox", { name: "Section 1: what goes in it" }).fill("What worked this week.");
  await editor.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByText("Template saved", { exact: true }).first()).toBeVisible();
  await expect(page.getByTestId("user-templates").getByText("Weekly retro")).toBeVisible();

  // Regenerate a meeting with it.
  await toMeeting(page);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Weekly retro" }).click();
  await page.getByRole("button", { name: "Regenerate notes" }).click();
  await expect(page.getByRole("heading", { name: "Went well" })).toBeVisible({ timeout: 15000 });
  await expect(page.locator("textarea", { hasText: "Notes for Went well" })).toBeVisible();

  // Rename the section: the same notes, the new title.
  await toSettings(page);
  await page.getByRole("button", { name: "Edit Weekly retro" }).click();
  const title = page.getByRole("textbox", { name: "Section 1: title" });
  await title.fill("Good things");
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByText("Template saved", { exact: true }).first()).toBeVisible();
  await toMeeting(page);
  await expect(page.getByRole("heading", { name: "Good things" })).toBeVisible();
  await expect(page.locator("textarea", { hasText: "Notes for Went well" })).toBeVisible();

  // Delete the template: every section of the notes is still there.
  await toSettings(page);
  await page.getByRole("button", { name: "Delete Weekly retro" }).click();
  await page.getByRole("button", { name: "Delete template" }).click();
  await expect(page.getByTestId("user-templates").getByText("Weekly retro")).toHaveCount(0);
  await toMeeting(page);
  await expect(page.getByRole("heading", { name: "Went well" })).toBeVisible();
  await expect(page.locator("textarea", { hasText: "Notes for Went well" })).toBeVisible();
});

test("draft from a description fills the editor; nothing is saved until Save", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await toSettings(page);
  await page.getByRole("button", { name: "New template" }).click();
  const editor = page.getByTestId("template-editor");
  await editor.getByRole("textbox", { name: "Describe your meetings" }).fill("weekly design critique with the product team");
  await editor.getByRole("button", { name: "Draft" }).click();
  await expect(editor.getByRole("textbox", { name: "Name" })).toHaveValue("Weekly design critique");
  await expect(editor.getByTestId("template-section")).toHaveCount(2);
  await editor.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByTestId("user-templates").getByRole("listitem")).toHaveCount(0);
  // Again, and this time keep it.
  await page.getByRole("button", { name: "New template" }).click();
  await editor.getByRole("textbox", { name: "Describe your meetings" }).fill("weekly design critique");
  await editor.getByRole("button", { name: "Draft" }).click();
  await expect(editor.getByTestId("template-section")).toHaveCount(2);
  await editor.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByTestId("user-templates").getByText("Weekly design critique")).toBeVisible();
});

test("a draft that must wait says so in words", async ({ page }) => {
  await page.goto("/?platform=win&askfail=noModel#/meetings");
  await toSettings(page);
  await page.getByRole("button", { name: "New template" }).click();
  const editor = page.getByTestId("template-editor");
  await editor.getByRole("textbox", { name: "Describe your meetings" }).fill("anything");
  await editor.getByRole("button", { name: "Draft" }).click();
  await expect(editor.getByText(/The notes model isn.t installed yet/)).toBeVisible();
});

test("a built-in template duplicates into an editable copy; Save needs a complete form", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await toSettings(page);
  await page.getByRole("button", { name: /^Duplicate Client/ }).click();
  const editor = page.getByTestId("template-editor");
  await expect(editor.getByRole("textbox", { name: "Name" })).toHaveValue(/ \(copy\)$/);
  await expect(editor.getByTestId("template-section")).toHaveCount(2);
  await editor.getByRole("textbox", { name: "Section 1: title" }).fill("");
  await expect(editor.getByRole("button", { name: "Save template" })).toBeDisabled();
  await editor.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByTestId("builtin-templates")).toBeVisible();
  // Cancelled: no copy was made.
  await expect(page.getByTestId("user-templates").getByRole("listitem")).toHaveCount(0);
});

test("axe finds nothing on the templates list and the editor", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await toSettings(page);
  const scan = async () => {
    const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  };
  await scan();
  await page.getByRole("button", { name: "New template" }).click();
  await scan();
});
