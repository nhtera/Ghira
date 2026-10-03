// SPDX-License-Identifier: Apache-2.0
// Settings → Languages: the meeting language and the custom vocabulary the
// final transcript is corrected towards (RT-14).
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Icon, Segmented } from "@ghi/ui";
import type { AppSettings, MeetingLanguage, Vocabulary } from "../../bindings";
import { APP_NAME, formatDate, formatTime, type Locale } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { addTerm, editTerm, type TermResult } from "./logic";
import { Card, Row, Switch, bigSegCls, inputCls, useFail, useSettings } from "./parts";

const vocabKey = ["vocabulary"] as const;
/** The sample shown under "Dates and times". */
const SAMPLE = new Date(2026, 8, 28, 14, 30);

export function LanguagesSection() {
  const { t, i18n } = useTranslation();
  const lang: Locale = i18n.language === "vi" ? "vi" : "en";
  const { settings, patch } = useSettings();
  return (
    <div className="flex flex-col">
      <Card>
        <Row label={t("settings.languages.meetingLanguages")} hint={t("onboarding.languages.body", { app: APP_NAME })}>
          {settings && (
            <Segmented<MeetingLanguage>
              label={t("settings.languages.meetingLanguages")}
              className={bigSegCls}
              value={settings.meetingLanguage}
              onChange={(v) => void patch({ meetingLanguage: v })}
              options={[
                { value: "en", label: t("onboarding.languages.english") },
                { value: "vi", label: t("onboarding.languages.vietnamese") },
                { value: "auto", label: t("onboarding.languages.both") },
              ]}
            />
          )}
        </Row>
        <Row label={t("settings.languages.notesLanguage")}>
          {settings && (
            <Segmented<AppSettings["notesLanguage"]>
              label={t("settings.languages.notesLanguage")}
              className={bigSegCls}
              value={settings.notesLanguage}
              onChange={(v) => void patch({ notesLanguage: v })}
              options={[
                { value: "meeting", label: t("settings.languages.sameAsMeeting") },
                { value: "en", label: t("settings.languages.english") },
                { value: "vi", label: t("settings.languages.vietnamese") },
              ]}
            />
          )}
        </Row>
        <Row label={t("settings.languages.dateFormat")} hint={`${formatDate(SAMPLE, lang)} · ${formatTime(SAMPLE, lang)}`} />
        <Row label={t("settings.languages.accentSearch")} hint={t("settings.languages.accentSearchHint")}>
          <Switch checked onChange={() => {}} label={t("settings.languages.accentSearch")} disabled />
        </Row>
      </Card>
      <VocabularyCard />
    </div>
  );
}

export function VocabularyCard() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const fail = useFail();
  const { data } = useQuery({
    queryKey: vocabKey,
    queryFn: async () => {
      const r = await ipc.commands.vocabulary();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  const [draft, setDraft] = useState("");
  const [message, setMessage] = useState<TermResult["status"] | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [editText, setEditText] = useState("");
  const [saving, setSaving] = useState(false);
  const addRef = useRef<HTMLInputElement>(null);

  const store = (v: Vocabulary) => queryClient.setQueryData(vocabKey, v);
  // Edits are built from the latest saved list and run one at a time, so two
  // quick removals can't save from the same stale list.
  const save = async (terms: string[]) => {
    setSaving(true);
    const r = await ipc.commands.setVocabulary(terms);
    setSaving(false);
    if (r.status === "ok") store(r.data);
    else fail(r.error);
    return r.status === "ok";
  };
  const latest = () => queryClient.getQueryData<Vocabulary>(vocabKey) ?? data;
  const submit = async () => {
    const cur = latest();
    if (!cur || saving) return;
    const res = addTerm(cur.terms, draft, cur.maxTerms);
    setMessage(res.status === "added" ? null : res.status);
    if (res.status !== "added") return;
    if (await save(res.terms)) setDraft("");
    addRef.current?.focus();
  };
  const commitEdit = async () => {
    const cur = latest();
    if (!cur || editing == null || saving) return;
    const res = editTerm(cur.terms, editing, editText);
    if (res.status === "duplicate") return setMessage("duplicate");
    setMessage(null);
    setEditing(null);
    if (res.terms.join("\n") !== cur.terms.join("\n")) await save(res.terms);
  };
  const forget = async (term: string) => {
    if (saving) return;
    setSaving(true);
    const r = await ipc.commands.ignoreLearnedTerm(term);
    setSaving(false);
    if (r.status === "ok") store(r.data);
    else fail(r.error);
  };

  const full = data ? data.terms.length >= data.maxTerms : false;
  return (
    <Card title={t("settings.languages.vocab.title")} hint={t("settings.languages.vocab.hint")} className="max-w-2xl">
      {data && (
        <>
          <div className="flex items-center gap-2">
            <input
              ref={addRef}
              className={`${inputCls} flex-1`}
              value={draft}
              maxLength={80}
              aria-label={t("settings.languages.vocab.add")}
              placeholder={t("settings.languages.vocab.placeholder")}
              disabled={full}
              onChange={(e) => {
                setDraft(e.target.value);
                setMessage(null);
              }}
              onKeyDown={(e) => e.key === "Enter" && !e.nativeEvent.isComposing && void submit()}
            />
            <Button size="lg" icon="add" onClick={() => void submit()} disabled={full || saving || !draft.trim()}>
              {t("settings.languages.vocab.addButton")}
            </Button>
            <span className="text-small min-w-12 text-right text-muted tabular-nums" data-testid="vocab-count">
              {data.terms.length}/{data.maxTerms}
            </span>
          </div>
          <span role="status" className="text-small min-h-4 text-warn">
            {(message === "duplicate" || message === "full") && t(`settings.languages.vocab.${message}`)}
          </span>
          {data.terms.length === 0 ? (
            <p className="text-small m-0 text-muted">{t("settings.languages.vocab.empty")}</p>
          ) : (
            <ul aria-label={t("settings.languages.vocab.title")} className="m-0 flex list-none flex-wrap gap-1.5 p-0">
              {data.terms.map((term, i) => (
                <li key={term} className="flex items-center gap-0.5 rounded-full border border-line2 bg-surface2 py-0.5 pr-0.5 pl-3 text-[13px]">
                  {editing === i ? (
                    <input
                      autoFocus
                      className={`${inputCls} h-6 w-40`}
                      value={editText}
                      aria-label={t("settings.languages.vocab.editTerm", {
                        term,
                      })}
                      onChange={(e) => setEditText(e.target.value)}
                      onBlur={() => void commitEdit()}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.nativeEvent.isComposing) void commitEdit();
                        if (e.key === "Escape") setEditing(null);
                      }}
                    />
                  ) : (
                    <>
                      <span>{term}</span>
                      <button
                        type="button"
                        className="grid size-6 place-items-center rounded-full text-muted hover:bg-sunk"
                        aria-label={t("settings.languages.vocab.editTerm", {
                          term,
                        })}
                        onClick={() => {
                          setEditing(i);
                          setEditText(term);
                        }}
                      >
                        <Icon name="edit" size={14} />
                      </button>
                    </>
                  )}
                  <button
                    type="button"
                    className="grid size-6 place-items-center rounded-full text-muted hover:bg-sunk"
                    aria-label={t("settings.languages.vocab.removeTerm", {
                      term,
                    })}
                    disabled={saving}
                    onClick={() => void save((latest()?.terms ?? data.terms).filter((x) => x !== term))}
                  >
                    <Icon name="close" size={14} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <h4 className="text-body m-0 mt-2 font-semibold">{t("settings.languages.vocab.learnedTitle")}</h4>
          <p className="text-small m-0 text-muted">{t("settings.languages.vocab.learnedHint")}</p>
          {data.learned.length === 0 ? (
            <p className="text-small m-0 text-muted">{t("settings.languages.vocab.learnedEmpty")}</p>
          ) : (
            <ul aria-label={t("settings.languages.vocab.learnedTitle")} className="m-0 flex list-none flex-wrap gap-1.5 p-0">
              {data.learned.map((term) => (
                <li key={term} className="flex items-center gap-0.5 rounded-full border border-line bg-surface py-0.5 pr-0.5 pl-3 text-[13px]">
                  <span>{term}</span>
                  <button
                    type="button"
                    className="grid size-6 place-items-center rounded-full text-muted hover:bg-sunk"
                    aria-label={t("settings.languages.vocab.removeTerm", {
                      term,
                    })}
                    disabled={saving}
                    onClick={() => void forget(term)}
                  >
                    <Icon name="close" size={14} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </Card>
  );
}
