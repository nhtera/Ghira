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
  draftTemplate: vi.fn(),
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

  it("duplicating opens an unsaved form with a localized copy name; only Save creates it", async () => {
    const user = userEvent.setup();
    commands.duplicateTemplate.mockImplementation((_id: string, language: string) =>
      ok({ name: "Standup", language, guidance: "", sections: [{ id: null, title: "Done", instruction: "What was done." }] }),
    );
    commands.createTemplate.mockImplementation((f: TemplateForm) => ok({ id: "user:t3", form: f }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "Duplicate Standup" }));
    expect(commands.duplicateTemplate).toHaveBeenCalledWith("standup", "en");
    let editor = within(await screen.findByTestId("template-editor"));
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Standup (copy)");
    // Cancel: nothing was created.
    await user.click(editor.getByRole("button", { name: "Cancel" }));
    expect(commands.createTemplate).not.toHaveBeenCalled();
    // Again, then Save: one template, with the copy name.
    await user.click(await screen.findByRole("button", { name: "Duplicate Standup" }));
    editor = within(await screen.findByTestId("template-editor"));
    await user.click(editor.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(commands.createTemplate).toHaveBeenCalledTimes(1));
    expect(commands.createTemplate.mock.calls[0]![0].name).toBe("Standup (copy)");
  });

  it("does not scold before the person has typed, then says why Save is off", async () => {
    const user = userEvent.setup();
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    expect(editor.queryByText("Give the template a name.")).toBeNull();
    expect((editor.getByRole("button", { name: "Save template" }) as HTMLButtonElement).disabled).toBe(true);
    await user.type(editor.getByRole("textbox", { name: "Section 1: title" }), "A");
    expect(editor.getByText("Give the template a name.")).toBeTruthy();
  });

  it("removing a section keeps the text of the others where it was typed", async () => {
    const user = userEvent.setup();
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    await user.click(editor.getByRole("button", { name: "Add a section" }));
    await user.type(editor.getByRole("textbox", { name: "Section 1: title" }), "First");
    await user.type(editor.getByRole("textbox", { name: "Section 2: title" }), "Second");
    await user.click(editor.getByRole("button", { name: "Remove section 1" }));
    expect((editor.getByRole("textbox", { name: "Section 1: title" }) as HTMLInputElement).value).toBe("Second");
    expect(editor.getAllByTestId("template-section")).toHaveLength(1);
  });

  it("a change in flight turns the other buttons off", async () => {
    const user = userEvent.setup();
    let done!: (v: unknown) => void;
    commands.deleteTemplate.mockReturnValue(new Promise((r) => (done = r)));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "Delete Retro" }));
    await user.click(screen.getByRole("button", { name: "Delete template" }));
    expect((screen.getByRole("button", { name: "Edit Retro" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Duplicate Standup" }) as HTMLButtonElement).disabled).toBe(true);
    done({ status: "ok", data: null });
    await waitFor(() => expect((screen.getByRole("button", { name: "Edit Retro" }) as HTMLButtonElement).disabled).toBe(false));
  });

  it("drafts from a description into the editor, with nothing saved; Save then creates it", async () => {
    const user = userEvent.setup();
    commands.draftTemplate.mockImplementation((_d: string, language: string) =>
      ok({ name: "Weekly retro", language, guidance: "A retro.", sections: [{ id: null, title: "Went well", instruction: "What worked." }, { id: null, title: "Went badly", instruction: "What did not." }] }),
    );
    commands.createTemplate.mockImplementation((f: TemplateForm) => ok({ id: "user:t9", form: f }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    const draft = editor.getByRole("button", { name: "Draft" }) as HTMLButtonElement;
    expect(draft.disabled).toBe(true);
    await user.type(editor.getByRole("textbox", { name: "Describe your meetings" }), "A weekly team retro");
    await user.click(draft);
    expect(commands.draftTemplate).toHaveBeenCalledWith("A weekly team retro", "en");
    await waitFor(() => expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Weekly retro"));
    expect(editor.getAllByTestId("template-section")).toHaveLength(2);
    expect(editor.getByText("Draft ready. Review it, change what you like, then save.")).toBeTruthy();
    // Drafting saved nothing.
    expect(commands.createTemplate).not.toHaveBeenCalled();
    await user.click(editor.getByRole("button", { name: "Save template" }));
    await waitFor(() => expect(commands.createTemplate).toHaveBeenCalledTimes(1));
    expect(commands.createTemplate.mock.calls[0]![0].sections.map((x: { id: string | null }) => x.id)).toEqual([null, null]);
  });

  it("a draft asks before it replaces what was typed; Cancel keeps it, Replace takes the draft", async () => {
    const user = userEvent.setup();
    commands.draftTemplate.mockImplementation((_d: string, language: string) =>
      ok({ name: "Drafted", language, guidance: "", sections: [{ id: null, title: "Drafted section", instruction: "Drafted line." }] }),
    );
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    await user.type(editor.getByRole("textbox", { name: "Name" }), "My own");
    await user.type(editor.getByRole("textbox", { name: "Describe your meetings" }), "anything");
    await user.click(editor.getByRole("button", { name: "Draft" }));
    expect(await editor.findByText("Replace what you have typed with the draft?")).toBeTruthy();
    // nothing changed yet
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("My own");
    await user.click(editor.getAllByRole("button", { name: "Cancel" }).find((b) => b.closest("[role=alertdialog],[data-confirm],div")!.textContent!.includes("Replace what"))!);
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("My own");
    expect(editor.queryByText("Replace what you have typed with the draft?")).toBeNull();
    await user.click(editor.getByRole("button", { name: "Draft" }));
    await user.click(await editor.findByRole("button", { name: "Replace" }));
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Drafted");
    expect((editor.getByRole("textbox", { name: "Section 1: title" }) as HTMLInputElement).value).toBe("Drafted section");
  });

  it("a draft that has to wait says why in words and leaves the form alone; editing has no draft box", async () => {
    const user = userEvent.setup();
    commands.draftTemplate.mockImplementation(() => Promise.resolve({ status: "error" as const, error: "busyNotes" }));
    renderSettings(<TemplatesSection />);
    await user.click(await screen.findByRole("button", { name: "New template" }));
    const editor = within(screen.getByTestId("template-editor"));
    await user.type(editor.getByRole("textbox", { name: "Describe your meetings" }), "x");
    await user.click(editor.getByRole("button", { name: "Draft" }));
    expect(await editor.findByText("Notes are being written. Try again in a moment.")).toBeTruthy();
    expect((editor.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("");
    await user.click(editor.getByRole("button", { name: "Cancel" }));
    await user.click(await screen.findByRole("button", { name: "Edit Retro" }));
    expect(screen.queryByTestId("template-draft")).toBeNull();
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
