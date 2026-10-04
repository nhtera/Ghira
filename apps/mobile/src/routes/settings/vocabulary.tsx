// SPDX-License-Identifier: Apache-2.0
// Settings → Custom vocabulary: names, products and jargon the final transcript
// is corrected towards, plus the names learned from speakers (removable).
// The core owns the list (and its 200-term cap); each change saves whole.
import { ListRow, ListSection } from "@ghi/ui";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Vocabulary } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useAction, useResource } from "../../features/settings/api";
import { Btn, ErrorLine, Field } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { addTerm, type TermResult } from "../../features/settings/vocab";

const load = async () => unwrap(await ipc.commands.vocabulary());

export function VocabularyScreen() {
  const { t } = useTranslation();
  const vocab = useResource(load);
  const action = useAction();
  const [draft, setDraft] = useState("");
  const [message, setMessage] = useState<TermResult["status"] | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const v = vocab.data;

  const apply = (next: () => Promise<Vocabulary>) =>
    action.run(async () => {
      vocab.set(await next());
      return true;
    });
  const submit = async () => {
    if (!v || action.busy) return;
    const res = addTerm(v.terms, draft, v.maxTerms);
    setMessage(res.status === "added" ? null : res.status);
    if (res.status !== "added") return;
    if (await apply(async () => unwrap(await ipc.commands.setVocabulary(res.terms)))) setDraft("");
    input.current?.focus();
  };
  const remove = (term: string) =>
    void apply(async () => unwrap(await ipc.commands.setVocabulary(v!.terms.filter((x) => x !== term))));
  const forget = (term: string) => void apply(async () => unwrap(await ipc.commands.ignoreLearnedTerm(term)));

  return (
    <Page title={t("mobile.settings.vocab.title")} back="settings" error={vocab.error} onRetry={vocab.reload}>
      {v && (
        <>
          <ErrorLine code={action.error} fallback="mobile.settings.saveFailed" />
          <ListSection header={t("mobile.settings.vocab.addHeader")} footer={t("mobile.settings.vocab.hint")}>
            <li className="flex flex-col gap-2 px-4 py-3">
              <form
                className="flex flex-col gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  void submit();
                }}
              >
                <Field
                  ref={input}
                  id="vocab-add"
                  label={t("mobile.settings.vocab.field")}
                  placeholder={t("mobile.settings.vocab.placeholder")}
                  value={draft}
                  maxLength={80}
                  autoCapitalize="none"
                  autoCorrect="off"
                  onChange={(e) => {
                    setDraft(e.target.value);
                    setMessage(null);
                  }}
                />
                <div className="flex flex-wrap items-center gap-3">
                  <Btn type="submit" tone="primary" disabled={action.busy || !draft.trim()}>
                    {t("mobile.settings.vocab.add")}
                  </Btn>
                  <span className="text-ios-footnote text-muted tabular-nums" data-testid="vocab-count">
                    {t("mobile.settings.vocab.count", { count: v.terms.length, max: v.maxTerms })}
                  </span>
                </div>
                {(message === "duplicate" || message === "full") && (
                  <p role="alert" className="text-ios-footnote m-0 text-rec-ink">
                    {t(`mobile.settings.vocab.${message}`)}
                  </p>
                )}
              </form>
            </li>
          </ListSection>

          {v.terms.length === 0 ? (
            <p className="text-ios-footnote mx-4 my-3 text-muted">{t("mobile.settings.vocab.empty")}</p>
          ) : (
            <ListSection header={t("mobile.settings.vocab.termsHeader")}>
              {v.terms.map((term) => (
                <ListRow
                  key={term}
                  title={term}
                  trailing={<RemoveButton label={t("mobile.settings.vocab.removeTerm", { term })} disabled={action.busy} onClick={() => remove(term)} />}
                />
              ))}
            </ListSection>
          )}

          <ListSection header={t("mobile.settings.vocab.learnedHeader")} footer={t("mobile.settings.vocab.learnedFooter")}>
            {v.learned.length === 0 ? (
              <ListRow title={t("mobile.settings.vocab.learnedEmpty")} />
            ) : (
              v.learned.map((term) => (
                <ListRow
                  key={term}
                  title={term}
                  trailing={<RemoveButton label={t("mobile.settings.vocab.removeTerm", { term })} disabled={action.busy} onClick={() => forget(term)} />}
                />
              ))
            )}
          </ListSection>
        </>
      )}
    </Page>
  );
}

function RemoveButton({ label, disabled, onClick }: { label: string; disabled: boolean; onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className="text-ios-body min-h-ios-target min-w-ios-target px-2 text-rec-ink disabled:opacity-50"
    >
      {t("mobile.settings.vocab.remove")}
    </button>
  );
}
