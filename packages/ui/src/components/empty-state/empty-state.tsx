// SPDX-License-Identifier: Apache-2.0
// Empty states for Library, Search, People, Ask and the Import queue.
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { Button } from "../../primitives/button";
import { usePlatform } from "../../platform/platform";
import { cn } from "../../utils/cn";

export type EmptyKind = "library" | "search" | "people" | "ask" | "import";

export type EmptyStateProps = {
  kind: EmptyKind;
  /** The text that found nothing (search). */
  query?: string;
  /** Library: Record call; Import: Choose files. */
  onPrimary?: () => void;
  /** Library: Import a file. */
  onSecondary?: () => void;
  /** Library: the record shortcut, under the buttons. */
  hint?: string;
  className?: string;
};

const ICON: Record<EmptyKind, IconName> = {
  library: "graphic_eq",
  search: "search_off",
  people: "group",
  ask: "forum",
  import: "inbox",
};

export function EmptyState({ kind, query, onPrimary, onSecondary, hint, className }: EmptyStateProps) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const copy = {
    library: { title: t("library.empty.title"), body: t("library.empty.body", { context: platform }), primary: t("library.empty.record"), secondary: t("library.empty.importFile") },
    search: { title: t("library.emptySearch.title", { query: query ?? "" }), body: t("library.emptySearch.body") },
    people: { title: t("people.empty.title"), body: t("people.empty.body") },
    ask: { title: t("ask.empty.title"), body: t("ask.empty.body") },
    import: { title: t("import.emptyTitle"), body: t("import.emptyBody"), primary: t("import.chooseFiles") },
  }[kind] as { title: string; body: string; primary?: string; secondary?: string };
  const library = kind === "library";
  return (
    <div data-kind={kind} className={cn("flex flex-col items-center px-4 py-6 text-center", library ? "gap-2.5" : "gap-2", className)}>
      <Icon name={ICON[kind]} size={library ? 40 : 32} className={library ? "text-accent" : "text-muted"} />
      <h3 className={cn("text-heading m-0 leading-snug", library ? "text-[18px]" : "text-[14px]")}>{copy.title}</h3>
      <p className={cn("m-0 text-muted", library ? "max-w-[420px] text-[13.5px]" : "text-small max-w-sm")}>{copy.body}</p>
      {(onPrimary || onSecondary) && (
        <div className={cn("flex flex-wrap justify-center gap-2", library ? "mt-2" : "mt-1")}>
          {onPrimary && copy.primary && (
            <Button variant="primary" size={library ? "lg" : "md"} onClick={onPrimary}>
              {copy.primary}
            </Button>
          )}
          {onSecondary && copy.secondary && (
            <Button size={library ? "lg" : "md"} onClick={onSecondary}>
              {copy.secondary}
            </Button>
          )}
        </div>
      )}
      {hint && <span className="mt-1.5 text-[12px] text-faint">{hint}</span>}
    </div>
  );
}
