// SPDX-License-Identifier: Apache-2.0
// The template editor (Settings → Templates): name, the language it is written
// in, an optional line on what the meetings are, and up to eight sections of a
// title and one line on what goes in. Section ids are the core's: an existing
// section keeps its id (`id` is sent back as it came) so renaming never
// detaches notes already written.
import { Button, Icon, InlineConfirm, Segmented } from "@ghi/ui";
import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TemplateForm } from "../../bindings";
import { inputCls } from "./parts";
import { MAX_DESCRIPTION, hasContent, MAX_GUIDANCE, MAX_INSTRUCTION, MAX_NAME, MAX_SECTIONS, MAX_SECTION_TITLE, formIssue } from "./template-form";

export function TemplateEditor({
  initial,
  isNew,
  saving,
  onSave,
  onCancel,
  onDraft,
}: {
  initial: TemplateForm;
  isNew: boolean;
  saving: boolean;
  onSave: (form: TemplateForm) => void;
  onCancel: () => void;
  /** Asks the local model for a draft of a new template; the form is filled in, never saved. */
  onDraft?: (description: string, language: string) => Promise<{ form: TemplateForm } | { error: string } | null>;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState(initial);
  // Each section row keeps its own key (not its position or id): removing one never moves another's text.
  const nextKey = useRef(initial.sections.length);
  const [keys, setKeys] = useState(() => initial.sections.map((_, i) => i));
  // The reason Save is off is only said once the person has started typing.
  const [touched, setTouched] = useState(false);
  const uid = useId();
  const [description, setDescription] = useState("");
  const [drafting, setDrafting] = useState(false);
  // What the last draft attempt said: "drafted", or a waiting state (busy, no model) in words.
  const [draftNote, setDraftNote] = useState<string | null>(null);
  // A draft that came back while the form already holds the person's own words: asks before replacing them.
  const [pending, setPending] = useState<TemplateForm | null>(null);
  const issue = formIssue(form);
  const set = (p: Partial<TemplateForm>) => {
    setTouched(true);
    setForm((f) => ({ ...f, ...p }));
  };
  const setSection = (i: number, p: Partial<TemplateForm["sections"][number]>) => set({ sections: form.sections.map((s, k) => (k === i ? { ...s, ...p } : s)) });
  const draft = async () => {
    if (!onDraft || drafting || !description.trim()) return;
    setDrafting(true);
    setDraftNote(null);
    const r = await onDraft(description, form.language);
    setDrafting(false);
    if (!r) return;
    if ("error" in r) return setDraftNote(r.error);
    // What was typed is not replaced without asking.
    if (hasContent(form)) return setPending(r.form);
    apply(r.form);
  };
  const apply = (next: TemplateForm) => {
    nextKey.current += next.sections.length;
    setKeys(next.sections.map((_, i) => nextKey.current - next.sections.length + i));
    setTouched(true);
    setForm(next);
    setPending(null);
    setDraftNote(t("settings.templates.editor.drafted"));
  };
  const addSection = () => {
    setKeys((k) => [...k, nextKey.current++]);
    set({ sections: [...form.sections, { id: null, title: "", instruction: "" }] });
  };
  const removeSection = (i: number) => {
    setKeys((k) => k.filter((_, n) => n !== i));
    set({ sections: form.sections.filter((_, n) => n !== i) });
  };

  return (
    <form
      aria-label={t(isNew ? "settings.templates.editor.titleNew" : "settings.templates.editor.titleEdit")}
      data-testid="template-editor"
      className="mt-4 flex flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (!issue && !saving) onSave(form);
      }}
    >
      <h3 className="m-0 text-[15px] font-semibold">{t(isNew ? "settings.templates.editor.titleNew" : "settings.templates.editor.titleEdit")}</h3>
      {isNew && onDraft && (
        <div data-testid="template-draft" className="flex flex-col gap-1.5 rounded-panel border border-line2 p-3">
          <label htmlFor={`${uid}-draft`} className="text-[13px] font-medium">
            {t("settings.templates.editor.draftLabel")}
          </label>
          <textarea
            id={`${uid}-draft`}
            value={description}
            rows={2}
            maxLength={MAX_DESCRIPTION}
            aria-describedby={`${uid}-dh`}
            onChange={(e) => setDescription(e.target.value)}
            className="min-w-0 resize-none rounded-ctl border border-line2 bg-surface p-2 text-[13.5px] text-ink focus-visible:outline-2 focus-visible:outline-accent"
          />
          <span id={`${uid}-dh`} className="text-[12px] text-muted">
            {t("settings.templates.editor.draftHint")}
          </span>
          {pending && (
            <InlineConfirm
              icon="warning"
              tone="warn"
              question={t("settings.templates.editor.replaceQuestion")}
              confirmLabel={t("settings.templates.editor.replace")}
              onConfirm={() => apply(pending)}
              onCancel={() => setPending(null)}
            />
          )}
          <div className="flex items-center gap-2">
            <Button size="sm" icon="auto_awesome" disabled={drafting || !!pending || !description.trim()} onClick={() => void draft()}>
              {t("settings.templates.editor.draftButton")}
            </Button>
            <span role="status" className="text-[12.5px] text-muted">
              {drafting ? t("settings.templates.editor.drafting") : draftNote}
            </span>
          </div>
        </div>
      )}
      <div className="flex flex-col gap-1">
        <label htmlFor={`${uid}-name`} className="text-[13px] font-medium">
          {t("settings.templates.editor.name")}
        </label>
        <input id={`${uid}-name`} value={form.name} maxLength={MAX_NAME} onChange={(e) => set({ name: e.target.value })} className={inputCls} />
      </div>
      <div className="flex items-center gap-3">
        <span className="text-[13px] font-medium">{t("settings.templates.editor.language")}</span>
        <Segmented<string>
          label={t("settings.templates.editor.language")}
          value={form.language}
          onChange={(language) => set({ language })}
          options={[
            { value: "en", label: t("settings.templates.editor.langEn") },
            { value: "vi", label: t("settings.templates.editor.langVi") },
          ]}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label htmlFor={`${uid}-guidance`} className="text-[13px] font-medium">
          {t("settings.templates.editor.guidance")}
        </label>
        <input id={`${uid}-guidance`} value={form.guidance} maxLength={MAX_GUIDANCE} onChange={(e) => set({ guidance: e.target.value })} aria-describedby={`${uid}-gh`} className={inputCls} />
        <span id={`${uid}-gh`} className="text-[12px] text-muted">
          {t("settings.templates.editor.guidanceHint")}
        </span>
      </div>
      <fieldset className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0">
        <legend className="mb-1 p-0 text-[13px] font-medium">{t("settings.templates.editor.sections")}</legend>
        <p className="m-0 text-[12px] text-muted">{t("settings.templates.editor.sectionsHint")}</p>
        {form.sections.map((s, i) => (
          <div key={keys[i]} data-testid="template-section" className="flex flex-col gap-1.5 rounded-panel border border-line2 p-2.5">
            <div className="flex items-center gap-2">
              <input
                aria-label={t("settings.templates.editor.sectionTitle", { number: i + 1 })}
                value={s.title}
                maxLength={MAX_SECTION_TITLE}
                onChange={(e) => setSection(i, { title: e.target.value })}
                className={`${inputCls} flex-1 font-semibold`}
              />
              <Button size="sm" variant="ghost" icon="close" aria-label={t("settings.templates.editor.removeSection", { number: i + 1 })} onClick={() => removeSection(i)} />
            </div>
            <input
              aria-label={t("settings.templates.editor.sectionInstruction", { number: i + 1 })}
              value={s.instruction}
              maxLength={MAX_INSTRUCTION}
              onChange={(e) => setSection(i, { instruction: e.target.value })}
              className={inputCls}
            />
            <span aria-hidden="true" className="self-end text-mono text-[11.5px] text-muted">
              {t("settings.templates.editor.count", { used: Array.from(s.instruction).length, max: MAX_INSTRUCTION })}
            </span>
          </div>
        ))}
        <Button size="sm" icon="add" disabled={form.sections.length >= MAX_SECTIONS} onClick={addSection} className="self-start">
          {t("settings.templates.editor.addSection")}
        </Button>
      </fieldset>
      {issue && touched && (
        <p role="status" className="m-0 flex items-center gap-1.5 text-[12.5px] text-muted">
          <Icon name="info" size={14} />
          {t(`settings.templates.editor.${issue}`)}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" variant="primary" disabled={!!issue || saving}>
          {t("settings.templates.editor.save")}
        </Button>
        <Button onClick={onCancel}>{t("common.cancel")}</Button>
      </div>
    </form>
  );
}
