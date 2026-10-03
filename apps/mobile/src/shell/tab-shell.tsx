// SPDX-License-Identifier: Apache-2.0
// The tab shell: the screen above a bottom tab bar (Meetings / Record /
// Search / Settings). Each screen owns its top safe-area inset (NavBar, or
// the record screen's own padding); the tab bar owns the bottom one.
import { TabBar, type TabBarItem } from "@ghi/ui";
import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

const TABS = [
  { id: "meetings", label: "mobile.tabs.meetings", icon: "format_list_bulleted", iconSelected: "format_list_bulleted_fill" },
  { id: "record", label: "mobile.tabs.record", icon: "mic", iconSelected: "mic_fill", emphasized: true },
  { id: "search", label: "mobile.tabs.search", icon: "search", iconSelected: "search_fill" },
  { id: "settings", label: "mobile.tabs.settings", icon: "settings", iconSelected: "settings_fill" },
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
