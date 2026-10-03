// SPDX-License-Identifier: Apache-2.0
// The tab shell: the screen above a bottom tab bar (Meetings / Record /
// Search / Settings). A plain bar for now; 16-F replaces it with @ghi/ui's
// TabBar (Record emphasized, 44 pt targets).
import { Link, Outlet } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

const TABS = [
  { to: "/meetings", label: "mobile.tabs.meetings" },
  { to: "/record", label: "mobile.tabs.record" },
  { to: "/search", label: "mobile.tabs.search" },
  { to: "/settings", label: "mobile.tabs.settings" },
] as const;

export function TabShell() {
  const { t } = useTranslation();
  return (
    <div className="tab-shell">
      <main>
        <Outlet />
      </main>
      <nav className="tab-bar" aria-label={t("mobile.shell.tabs")}>
        {TABS.map((tab) => (
          <Link key={tab.to} to={tab.to}>
            {t(tab.label)}
          </Link>
        ))}
      </nav>
    </div>
  );
}
