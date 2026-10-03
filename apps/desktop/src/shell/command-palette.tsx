// SPDX-License-Identifier: Apache-2.0
// ⌘K / Ctrl+K: navigation plus actions (design rationale #5). Ranking is ours
// (palette-rank.ts); cmdk provides the listbox and keyboard handling.
import { useNavigate } from "@tanstack/react-router";
import { Command } from "cmdk";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, resolveTheme, shortcutLabel, usePlatform, type IconName } from "@ghi/ui";
import library from "@ghi/ui/mocks/library.json";
import { ipc } from "../ipc";
import { isActive, useLive } from "../state/live";
import { usePrefs } from "../state/prefs";
import { useUi } from "../state/ui";
import { useAppActions } from "./actions";
import { SHORTCUTS } from "./shortcuts";
import { rankPalette, type PaletteGroup, type PaletteItem } from "./palette-rank";

type Entry = PaletteItem & { icon: IconName; run: () => void };

export function CommandPalette() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const open = useUi((s) => s.paletteOpen);
  const setOpen = useUi((s) => s.setPaletteOpen);
  const recording = isActive(useLive((s) => s.state));
  const { theme, setTheme } = usePrefs();
  const { startRecording, stopRecording, mark } = useAppActions();
  const [query, setQuery] = useState("");

  const entries = useMemo<Entry[]>(() => {
    const go = (id: string, label: string, icon: IconName, to: () => void): Entry => ({ id, group: "goTo", label, icon, run: to });
    const dark = resolveTheme(theme) === "dark";
    return [
      go("go-meetings", t("nav.meetings"), "event_note", () => void navigate({ to: "/meetings" })),
      ...(recording ? [go("go-live", t("nav.live"), "graphic_eq", () => void navigate({ to: "/live" }))] : []),
      go("go-people", t("nav.people"), "group", () => void navigate({ to: "/people" })),
      go("go-ask", t("nav.ask"), "forum", () => void navigate({ to: "/ask" })),
      go("go-import", t("nav.import"), "upload_file", () => void navigate({ to: "/import" })),
      go("go-settings", t("nav.settings"), "settings", () => void navigate({ to: "/settings/$section", params: { section: "general" } })),
      ...(recording
        ? [
            { id: "stop", group: "actions" as const, label: t("palette.stopRecording"), icon: "stop" as const, hint: shortcutLabel(SHORTCUTS.toggleRecording, platform), run: () => void stopRecording() },
            { id: "mark", group: "actions" as const, label: t("live.mark"), icon: "star" as const, hint: shortcutLabel(SHORTCUTS.mark, platform), run: () => void mark() },
          ]
        : [
            { id: "rec-call", group: "actions" as const, label: t("tray.recordCall"), icon: "videocam" as const, hint: shortcutLabel(SHORTCUTS.toggleRecording, platform), run: () => void startRecording("call") },
            { id: "rec-room", group: "actions" as const, label: t("tray.recordRoom"), icon: "groups" as const, run: () => void startRecording("room") },
          ]),
      { id: "import-file", group: "actions", label: t("library.empty.importFile"), icon: "upload_file", run: () => void navigate({ to: "/import" }) },
      {
        id: "theme",
        group: "actions",
        label: dark ? t("palette.lightTheme") : t("palette.darkTheme"),
        icon: dark ? "light_mode" : "dark_mode",
        run: () => setTheme(dark ? "light" : "dark"),
      },
      // Recent meetings come from the store in phase 10; samples only on the mock core.
      ...(ipc.kind === "mock" ? library.rows : []).map((r, i) => ({
        id: `meeting-${i}`,
        group: "recent" as const,
        label: r.ti,
        icon: "description" as const,
        hint: r.dur,
        run: () => void navigate({ to: "/meetings/$id/$tab", params: { id: `sample-${i}`, tab: "notes" } }),
      })),
    ];
  }, [t, navigate, recording, theme, setTheme, platform, startRecording, stopRecording, mark]);

  const groups = rankPalette(query, entries, (q) => ({
    id: "ask",
    group: "actions",
    label: t("palette.ask", { query: q }),
    hint: "↵",
  }));
  const byId = new Map(entries.map((e) => [e.id, e]));
  const heading: Record<PaletteGroup, string> = {
    goTo: t("palette.goTo"),
    actions: t("palette.actions"),
    recent: t("tray.recent"),
    meetings: t("nav.meetings"),
  };

  const close = () => {
    setOpen(false);
    setQuery("");
  };
  const select = (id: string) => {
    close();
    if (id === "ask") void navigate({ to: "/ask" });
    else byId.get(id)?.run();
  };

  return (
    <Command.Dialog
      open={open}
      onOpenChange={(o) => (o ? setOpen(true) : close())}
      label={t("shell.commandPalette")}
      shouldFilter={false}
      loop
      overlayClassName="fixed inset-0 z-40 bg-[var(--scrim)]"
      contentClassName="fixed top-[84px] left-1/2 z-50 w-[min(600px,calc(100vw-32px))] -translate-x-1/2 overflow-hidden rounded-panel border border-line2 bg-surface text-ink shadow-float"
    >
      <div className="flex h-[54px] items-center gap-2.5 border-b border-line px-4">
        <Icon name="search" size={20} className="text-faint" />
        <Command.Input
          value={query}
          onValueChange={setQuery}
          placeholder={t("palette.placeholder")}
          className="h-12 flex-1 border-0 bg-transparent text-[15px] text-ink outline-none placeholder:text-faint"
        />
        <kbd aria-hidden className="rounded-[5px] border border-line2 px-1.5 py-0.5 font-mono text-[11px] text-faint">
          {t("palette.esc")}
        </kbd>
      </div>
      <Command.List className="max-h-[420px] overflow-auto px-1.5 pt-1 pb-2">
        <Command.Empty className="text-small px-3 py-6 text-center text-muted">{t("palette.noMatches")}</Command.Empty>
        {groups.map((g) => (
          <Command.Group
            key={g.group}
            heading={heading[g.group]}
            className="[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pt-2.5 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:tracking-[.06em] [&_[cmdk-group-heading]]:text-faint [&_[cmdk-group-heading]]:uppercase"
          >
            {g.items.map((it) => {
              const e = byId.get(it.id);
              return (
                <Command.Item
                  key={it.id}
                  value={it.id}
                  onSelect={select}
                  className="group flex h-10 cursor-default items-center gap-2.5 rounded-lg px-2.5 text-[13.5px] data-[selected=true]:bg-accent-soft"
                >
                  <Icon name={e?.icon ?? "forum"} size={19} className="text-muted" />
                  <span className="flex-1 truncate">{it.label}</span>
                  {it.hint && <span className="font-mono text-[11.5px] text-faint">{it.hint}</span>}
                </Command.Item>
              );
            })}
          </Command.Group>
        ))}
      </Command.List>
      <p className="m-0 flex h-9 items-center gap-2 border-t border-line px-3.5 text-[11.5px] text-faint">
        <Icon name="lock" size={14} className="text-accent" />
        <span className="flex-1">{t("privacy.local")}</span>
        {t("palette.hint")}
      </p>
    </Command.Dialog>
  );
}
