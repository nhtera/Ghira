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
  className?: string;
};

const ICON: Record<EmptyKind, IconName> = {
  library: "graphic_eq",
  search: "search_off",
  people: "group",
  ask: "forum",
  import: "inbox",
};

export function EmptyState({ kind, query, onPrimary, onSecondary, className }: EmptyStateProps) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const copy = {
    library: { title: t("library.empty.title"), body: t("library.empty.body", { context: platform }), primary: t("library.empty.record"), secondary: t("library.empty.importFile") },
    search: { title: t("library.emptySearch.title", { query: query ?? "" }), body: t("library.emptySearch.body") },
    people: { title: t("people.empty.title"), body: t("people.empty.body") },
    ask: { title: t("ask.empty.title"), body: t("ask.empty.body") },
    import: { title: t("import.emptyTitle"), body: t("import.emptyBody"), primary: t("import.chooseFiles") },
  }[kind] as { title: string; body: string; primary?: string; secondary?: string };
  return (
    <div data-kind={kind} className={cn("flex flex-col items-center gap-2 px-4 py-6 text-center", className)}>
      <Icon name={ICON[kind]} size={32} className={kind === "library" ? "text-accent" : "text-muted"} />
      <h3 className="text-heading m-0 text-[14px] leading-snug">{copy.title}</h3>
      <p className="text-small m-0 max-w-sm text-muted">{copy.body}</p>
      {(onPrimary || onSecondary) && (
        <div className="mt-1 flex flex-wrap justify-center gap-2">
          {onPrimary && copy.primary && (
            <Button variant="primary" size="md" onClick={onPrimary}>
              {copy.primary}
            </Button>
          )}
          {onSecondary && copy.secondary && <Button onClick={onSecondary}>{copy.secondary}</Button>}
        </div>
      )}
    </div>
  );
}
