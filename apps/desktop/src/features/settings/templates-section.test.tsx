// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TemplateForm, UserTemplateView } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  listTemplates: vi.fn(),
  userTemplates: vi.fn(),
  createTemplate: vi.fn(),
  updateTemplate: vi.fn(),
  deleteTemplate: vi.fn(),
  duplicateTemplate: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderSettings } from "./test-utils";
import { TemplatesSection } from "./templates-section";

const retro: UserTemplateView = {
  id: "user:t1",
  form: { name: "Retro", language: "en", guidance: "", sections: [{ id: "went_well", title: "Went well", instruction: "What worked." }] },
};

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.listTemplates.mockImplementation(() =>
    ok([
      { id: "general", name: "General meeting", sections: [] },
      { id: "standup", name: "Standup", sections: [{ id: "done", titleEn: "Done", titleVi: "Đã làm" }] },
    ]),
  );
  commands.userTemplates.mockImplementation(() => ok([retro]));
});
afterEach(cleanup);

describe("TemplatesSection", () => {
  it("lists the built-in templates with Duplicate and yours with Edit and Delete", async () => {
    renderSettings(<TemplatesSection />);
    await screen.findByText("Retro");
    const mine = within(screen.getByTestId("user-templates"));
    expect(mine.getByText("1 section")).toBeTruthy();
    const builtin = within(screen.getByTestId("builtin-templates"));
    expect(await builtin.findByText("Done")).toBeTruthy();
    expect(builtin.getByRole("button", { name: "Duplicate Standup" })).toBeTruthy();
    // built-in ones cannot be edited or deleted
    expect(builtin.queryByRole("button", { name: /Edit|Delete/ })).toBeNull();
  });

  it("creates a template from the form; Save waits for a name and complete sections", async () => {
    const user = userEvent.setup();
    commands.createTemplate.mockImplementation((f: TemplateForm) => ok({ id: "user:t2", form: f }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    const save = editor.getByRole("button", { name: "Save template" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(editor.getByText("Give the template a name.")).toBeTruthy();
    await user.type(editor.getByRole("textbox", { name: "Name" }), "Weekly review");
    expect(save.disabled).toBe(true);
    expect(editor.getByText(/needs a title/)).toBeTruthy();
    await user.type(editor.getByRole("textbox", { name: "Section 1: title" }), "Risks");
    await user.type(editor.getByRole("textbox", { name: "Section 1: what goes in it" }), "What could go wrong.");
    expect(save.disabled).toBe(false);
    await user.click(save);
    await waitFor(() => expect(commands.createTemplate).toHaveBeenCalledTimes(1));
    expect(commands.createTemplate.mock.calls[0]![0]).toEqual({
      name: "Weekly review",
      language: "en",
      guidance: "",
      sections: [{ id: null, title: "Risks", instruction: "What could go wrong." }],
    });
    // back to the list, and the menus' list was asked again
    expect(await screen.findByTestId("user-templates")).toBeTruthy();
  });

  it("caps sections at eight and the instruction at 200 characters", async () => {
    const user = userEvent.setup();
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    const add = editor.getByRole("button", { name: "Add a section" }) as HTMLButtonElement;
    for (let i = 0; i < 7; i++) await user.click(add);
    expect(editor.getAllByTestId("template-section")).toHaveLength(8);
    expect(add.disabled).toBe(true);
    expect((editor.getByRole("textbox", { name: "Section 1: what goes in it" }) as HTMLInputElement).maxLength).toBe(200);
    await user.click(editor.getByRole("button", { name: "Remove section 8" }));
    expect(add.disabled).toBe(false);
  });

  it("editing sends the sections' ids back, so a rename keeps its notes", async () => {
    const user = userEvent.setup();
    commands.updateTemplate.mockImplementation((id: string, f: TemplateForm) => ok({ id, form: f }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "Edit Retro" }));
    const title = screen.getByRole("textbox", { name: "Section 1: title" });
    await user.clear(title);
    await user.type(title, "Good things");
    await user.click(screen.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(commands.updateTemplate).toHaveBeenCalledTimes(1));
    const [id, form] = commands.updateTemplate.mock.calls[0]!;
    expect(id).toBe("user:t1");
    expect(form.sections[0]).toEqual({ id: "went_well", title: "Good things", instruction: "What worked." });
  });

  it("deleting asks first and says the notes keep their sections", async () => {
    const user = userEvent.setup();
    commands.deleteTemplate.mockImplementation(() => ok(null));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "Delete Retro" }));
    expect(screen.getByText(/Notes already written with it keep all their sections/)).toBeTruthy();
    expect(commands.deleteTemplate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Delete template" }));
    await waitFor(() => expect(commands.deleteTemplate).toHaveBeenCalledWith("user:t1"));
  });

  it("duplicating a built-in one opens the copy in the editor, in the interface language", async () => {
    const user = userEvent.setup();
    commands.duplicateTemplate.mockImplementation((_id: string, language: string) =>
      ok({ id: "user:t3", form: { name: "Standup (copy)", language, guidance: "", sections: [{ id: "done", title: "Done", instruction: "What was done." }] } }),
    );
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "Duplicate Standup" }));
    expect(commands.duplicateTemplate).toHaveBeenCalledWith("standup", "en");
    const editor = within(await screen.findByTestId("template-editor"));
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Standup (copy)");
  });

  it("a refused save stays in the editor and says why", async () => {
    const user = userEvent.setup();
    commands.createTemplate.mockImplementation(() => Promise.resolve({ status: "error" as const, error: "at most 20 templates: delete one first" }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    await user.type(editor.getByRole("textbox", { name: "Name" }), "X");
    await user.type(editor.getByRole("textbox", { name: "Section 1: title" }), "A");
    await user.type(editor.getByRole("textbox", { name: "Section 1: what goes in it" }), "B");
    fireEvent.click(editor.getByRole("button", { name: "Save template" }));
    expect((await screen.findAllByText(/at most 20 templates/)).length).toBeGreaterThan(0);
    expect(screen.getByTestId("template-editor")).toBeTruthy();
  });
});
