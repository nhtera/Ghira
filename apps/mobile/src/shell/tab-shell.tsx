// SPDX-License-Identifier: Apache-2.0
// The tab shell: the screen above a bottom tab bar (Meetings / Record /
// Search / Settings). Each screen owns its top safe-area inset (NavBar, or
// the record screen's own padding); the tab bar owns the bottom one.
import { setNoticeSlot } from "../features/import-inbox/inbox-bus";
import { TabBar, type TabBarItem } from "@ghi/ui";
import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

const TABS = [
  { id: "meetings", label: "mobile.tabs.meetings", icon: "event_note" },
  { id: "record", label: "mobile.tabs.record", icon: "radio_button_checked", emphasized: true },
  { id: "search", label: "mobile.tabs.search", icon: "search" },
  { id: "settings", label: "mobile.tabs.settings", icon: "settings" },
] as const;

export function TabShell() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // /meetings/<id> belongs to the Meetings tab, /settings/about to Settings.
  const value = TABS.find((tab) => pathname === `/${tab.id}` || pathname.startsWith(`/${tab.id}/`))?.id ?? "meetings";
  const items: TabBarItem[] = TABS.map(({ label, ...tab }) => ({ ...tab, label: t(label) }));
  return (
    <div className="tab-shell">
      {/* Notices (files waiting to import): in the flow above the screen. It takes the status bar's inset itself, so the screen below drops its own (styles.css), and wears the screen's background: Settings is the grey app background, the others are white. */}
      <div ref={setNoticeSlot} data-notices className={`shrink-0 pt-safe empty:hidden ${value === "settings" ? "bg-bg" : "bg-surface"}`} />
      <main>
        <Outlet />
      </main>
      <TabBar
        label={t("mobile.shell.tabs")}
        items={items}
        value={value}
        onChange={(id) => void navigate({ to: `/${id}` })}
      />
    </div>
  );
}
