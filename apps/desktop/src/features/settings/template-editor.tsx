// SPDX-License-Identifier: Apache-2.0
// The template editor (Settings → Templates): name, the language it is written
// in, an optional line on what the meetings are, and up to eight sections of a
// title and one line on what goes in. Section ids are the core's: an existing
// section keeps its id (`id` is sent back as it came) so renaming never
// detaches notes already written.
import { Button, Icon, Segmented } from "@ghi/ui";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TemplateForm } from "../../bindings";
import { inputCls } from "./parts";
import { MAX_GUIDANCE, MAX_INSTRUCTION, MAX_NAME, MAX_SECTIONS, MAX_SECTION_TITLE, formIssue } from "./template-form";

export function TemplateEditor({
  initial,
  isNew,
  saving,
  onSave,
  onCancel,
}: {
  initial: TemplateForm;
  isNew: boolean;
  saving: boolean;
  onSave: (form: TemplateForm) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const [form, setForm] = useState(initial);
  const uid = useId();
  const issue = formIssue(form);
  const set = (p: Partial<TemplateForm>) => setForm((f) => ({ ...f, ...p }));
  const setSection = (i: number, p: Partial<TemplateForm["sections"][number]>) => set({ sections: form.sections.map((s, k) => (k === i ? { ...s, ...p } : s)) });

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
      <div className="flex flex-col gap-1">
        <label htmlFor={`${uid}-name`} className="text-[13px] font-medium">
          {t("settings.templates.editor.name")}
        </label>
        <input id={`${uid}-name`} value={form.name} maxLength={MAX_NAME} onChange={(e) => set({ name: e.target.value })} className={inputCls} />
      </div>
      <div className="flex items-center gap-3">
        <span id={`${uid}-lang`} className="text-[13px] font-medium">
          {t("settings.templates.editor.language")}
        </span>
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
          <div key={s.id ?? `new-${i}`} data-testid="template-section" className="flex flex-col gap-1.5 rounded-panel border border-line2 p-2.5">
            <div className="flex items-center gap-2">
              <input
                aria-label={t("settings.templates.editor.sectionTitle", { number: i + 1 })}
                value={s.title}
                maxLength={MAX_SECTION_TITLE}
                onChange={(e) => setSection(i, { title: e.target.value })}
                className={`${inputCls} flex-1 font-semibold`}
              />
              <Button size="sm" variant="ghost" icon="close" aria-label={t("settings.templates.editor.removeSection", { number: i + 1 })} onClick={() => set({ sections: form.sections.filter((_, k) => k !== i) })} />
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
        <Button size="sm" icon="add" disabled={form.sections.length >= MAX_SECTIONS} onClick={() => set({ sections: [...form.sections, { id: null, title: "", instruction: "" }] })} className="self-start">
          {t("settings.templates.editor.addSection")}
        </Button>
      </fieldset>
      {issue && (
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
