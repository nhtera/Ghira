// SPDX-License-Identifier: Apache-2.0
// Sidebar (brief §5): the ⌘K button, Meetings, Live (only while recording,
// with a red dot), People, Ask, Import; Settings and the on-device footer at
// the bottom. Icons only in a compact window.
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Icon, cn, shortcutLabel, usePlatform, type IconName } from "@ghi/ui";
import shell from "@ghi/ui/mocks/shell.json";
import { ipc } from "../ipc";
import { isActive, useLive } from "../state/live";
import { usePrefs } from "../state/prefs";
import { useUi } from "../state/ui";
import { SHORTCUTS } from "./shortcuts";

type NavItem = { to: "/meetings" | "/live" | "/people" | "/ask" | "/import"; label: string; icon: IconName; dot?: boolean };

const item =
  "flex h-9 items-center gap-2.5 rounded-ctl px-2.5 text-[13.5px] font-medium text-muted no-underline hover:bg-sunk " +
  "data-[status=active]:bg-surface data-[status=active]:text-ink data-[status=active]:shadow-[0_0_0_1px_var(--line)]";

export function Sidebar({ compact }: { compact: boolean }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const recording = isActive(useLive((s) => s.state));
  const openPalette = useUi((s) => s.setPaletteOpen);
  const lang = usePrefs((s) => s.language);
  const nav: NavItem[] = [
    { to: "/meetings", label: t("nav.meetings"), icon: "event_note" },
    ...(recording ? [{ to: "/live" as const, label: t("nav.live"), icon: "graphic_eq" as const, dot: true }] : []),
    { to: "/people", label: t("nav.people"), icon: "group" },
    { to: "/ask", label: t("nav.ask"), icon: "forum" },
    { to: "/import", label: t("nav.import"), icon: "upload_file" },
  ];
  return (
    <nav aria-label={t("shell.mainNav")} className="flex min-h-0 flex-col gap-0.5 border-r border-line bg-surface2 px-2.5 py-3">
      <button
        type="button"
        onClick={() => openPalette(true)}
        aria-label={compact ? t("nav.jump") : undefined}
        className="mb-2.5 flex h-[34px] items-center gap-2 rounded-ctl border border-line bg-surface px-2.5 text-[13px] text-muted"
      >
        <Icon name="search" size={17} />
        {!compact && (
          <>
            <span className="min-w-0 flex-1 truncate text-left">{t("nav.jump")}</span>
            <kbd className="font-mono text-[11px] text-muted">{shortcutLabel(SHORTCUTS.commandPalette, platform)}</kbd>
          </>
        )}
      </button>
      {nav.map((n) => (
        <Link key={n.to} to={n.to} className={item} aria-label={compact ? n.label : undefined}>
          <Icon name={n.icon} size={19} />
          {!compact && <span className="flex-1">{n.label}</span>}
          {n.dot && <span aria-hidden className={cn("size-2 rounded-full bg-rec", compact && "absolute ml-4 -mt-4")} />}
        </Link>
      ))}
      <div className="flex-1" />
      <Link
        to="/settings/$section"
        params={{ section: "general" }}
        className={item}
        aria-label={compact ? t("nav.settings") : undefined}
        activeOptions={{ includeSearch: false }}
      >
        <Icon name="settings" size={19} />
        {!compact && t("nav.settings")}
      </Link>
      {!compact && (
        <div className="mt-1.5 grid gap-0.5 border-t border-line px-2.5 pt-2.5 pb-0.5 text-[12px] text-muted">
          <span className="flex items-center gap-1.5 font-medium text-ink">
            <Icon name="lock" size={15} className="text-accent" />
            {t("nav.onDevice", { context: platform })}
          </span>
          {/* Tier and storage come from the core in phase 10; sample values only on the mock. */}
          {ipc.kind === "mock" && (
            <>
              <span>{t("nav.preset", shell.preset[lang])}</span>
              <span>{t("nav.storage", shell.storage[lang])}</span>
            </>
          )}
        </div>
      )}
    </nav>
  );
}
