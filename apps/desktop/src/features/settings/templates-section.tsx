// SPDX-License-Identifier: Apache-2.0
// Settings → Templates: the built-in templates (read-only, with Duplicate) and
// yours (edit, delete). Yours live on this device only (not synced); a
// template decides the sections of the notes written with it, and notes that
// already exist keep every section whatever happens to the template.
import { Button, InlineConfirm, useToast, usePlatform } from "@ghi/ui";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TemplateForm, UserTemplateView } from "../../bindings";
import { ipc } from "../../ipc";
import { useTemplates } from "../../state/meeting-queries";
import { templateName } from "../meeting/template-names";
import { Card, Note, useFail } from "./parts";
import { TemplateEditor } from "./template-editor";
import { MAX_TEMPLATES, emptyForm } from "./template-form";

const userKey = ["user-templates"] as const;

type Editing = { id: string | null; form: TemplateForm } | null;

export function TemplatesSection() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const { show } = useToast();
  const fail = useFail();
  const client = useQueryClient();
  const language = i18n.language === "vi" ? "vi" : "en";
  const builtins = useTemplates();
  const mine = useQuery({
    queryKey: userKey,
    queryFn: async (): Promise<UserTemplateView[]> => {
      const r = await ipc.commands.userTemplates();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  const [editing, setEditing] = useState<Editing>(null);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const yours = mine.data ?? [];
  const atLimit = yours.length >= MAX_TEMPLATES;

  // The menus read the same list, and the notes' section titles come from it: read them all again.
  const changed = () => Promise.all([client.invalidateQueries({ queryKey: userKey }), client.invalidateQueries({ queryKey: ["templates"] }), client.invalidateQueries({ queryKey: ["meeting"] })]);

  const save = async (form: TemplateForm) => {
    setSaving(true);
    const r = editing?.id ? await ipc.commands.updateTemplate(editing.id, form) : await ipc.commands.createTemplate(form);
    setSaving(false);
    if (r.status === "error") return fail(r.error);
    setEditing(null);
    show({ tone: "success", title: t("settings.templates.saved") });
    void changed();
  };
  const duplicate = async (id: string) => {
    const r = await ipc.commands.duplicateTemplate(id, language);
    if (r.status === "error") return fail(r.error);
    void changed();
    setEditing({ id: r.data.id, form: r.data.form });
  };
  const remove = async (id: string) => {
    setDeleting(null);
    const r = await ipc.commands.deleteTemplate(id);
    if (r.status === "error") return fail(r.error);
    show({ tone: "success", title: t("settings.templates.deleted") });
    void changed();
  };

  const sectionCount = (n: number) => t("settings.templates.sections", { count: n });

  return (
    <div className="flex flex-col">
      <p className="m-0 mt-1 text-[12.5px] text-muted">{t("settings.templates.hint", { context: platform })}</p>
      {editing ? (
        <TemplateEditor key={editing.id ?? "new"} initial={editing.form} isNew={editing.id == null} saving={saving} onSave={(f) => void save(f)} onCancel={() => setEditing(null)} />
      ) : (
        <>
          <Card title={t("settings.templates.yours")}>
            {yours.length === 0 && <Note>{t("settings.templates.none")}</Note>}
            <ul data-testid="user-templates" className="m-0 flex list-none flex-col p-0">
              {yours.map((u) => (
                <li key={u.id} data-row data-template={u.id} className="flex items-center gap-3 border-b border-line py-3">
                  {deleting === u.id ? (
                    <InlineConfirm question={t("settings.templates.deleteQuestion", { name: u.form.name })} confirmLabel={t("settings.templates.deleteConfirm")} onConfirm={() => void remove(u.id)} onCancel={() => setDeleting(null)} />
                  ) : (
                    <>
                      <div className="min-w-0 flex-1">
                        <div className="text-[14px] font-medium">{u.form.name}</div>
                        <div className="text-[12.5px] text-muted">{sectionCount(u.form.sections.length)}</div>
                      </div>
                      <Button size="sm" aria-label={t("settings.templates.editOf", { name: u.form.name })} onClick={() => setEditing({ id: u.id, form: u.form })}>
                        {t("settings.templates.edit")}
                      </Button>
                      <Button size="sm" variant="ghost" aria-label={t("settings.templates.duplicateOf", { name: u.form.name })} disabled={atLimit} onClick={() => void duplicate(u.id)}>
                        {t("settings.templates.duplicate")}
                      </Button>
                      <Button size="sm" variant="ghost" icon="delete" aria-label={t("settings.templates.deleteOf", { name: u.form.name })} onClick={() => setDeleting(u.id)} />
                    </>
                  )}
                </li>
              ))}
            </ul>
            <div className="flex items-center gap-3 pt-3">
              <Button variant="primary" icon="add" disabled={atLimit} onClick={() => setEditing({ id: null, form: emptyForm(language) })}>
                {t("settings.templates.new")}
              </Button>
              {atLimit && <span className="text-[12.5px] text-muted">{t("settings.templates.limit")}</span>}
            </div>
          </Card>
          <Card title={t("settings.templates.builtin")}>
            <ul data-testid="builtin-templates" className="m-0 flex list-none flex-col p-0">
              {(builtins.data ?? [])
                .filter((x) => !x.id.startsWith("user:"))
                .map((x) => (
                  <li key={x.id} data-row className="flex items-center gap-3 border-b border-line py-3">
                    <div className="min-w-0 flex-1">
                      <div className="text-[14px] font-medium">{templateName(x.id, t, x.name)}</div>
                      <div className="text-[12.5px] text-muted">{x.sections.length ? x.sections.map((s) => (language === "vi" ? s.titleVi : s.titleEn)).join(" · ") : sectionCount(0)}</div>
                    </div>
                    <Button size="sm" aria-label={t("settings.templates.duplicateOf", { name: templateName(x.id, t, x.name) })} disabled={atLimit} onClick={() => void duplicate(x.id)}>
                      {t("settings.templates.duplicate")}
                    </Button>
                  </li>
                ))}
            </ul>
          </Card>
        </>
      )}
    </div>
  );
}
