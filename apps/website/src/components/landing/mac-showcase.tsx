// SPDX-License-Identifier: Apache-2.0

import { type KeyboardEvent, useRef, useState } from "react";
import { landing } from "@/content/landing";
import { ThemedShot } from "./themed-shot";

const t = landing.apps;
const TABS = ["live", "notes"] as const;
type Tab = (typeof TABS)[number];
const SCREEN = { live: "desk-live", notes: "desk-notes" } as const;

/** The macOS window's red, yellow and green dots, as a picture (decorative). */
function TrafficLights() {
  return (
    <div className="mac-lights" aria-hidden="true">
      {["#FF5F57", "#FEBC2E", "#28C840"].map((fill) => (
        <svg key={fill} viewBox="0 0 12 12" focusable="false">
          <circle cx="6" cy="6" r="6" fill={fill} />
        </svg>
      ))}
    </div>
  );
}

/**
 * The Mac app's two screens as tabs (During a call / After the call):
 * arrow keys, Home and End move between them, and only the selected tab is
 * in the tab order. The caption follows the tab. Both panels are in the page
 * (the other is hidden), so it reads fully without JavaScript.
 */
export function MacShowcase() {
  const [tab, setTab] = useState<Tab>("live");
  const buttons = useRef<Record<Tab, HTMLButtonElement | null>>({ live: null, notes: null });

  function onKeyDown(e: KeyboardEvent) {
    const at = TABS.indexOf(tab);
    const next =
      e.key === "ArrowRight" || e.key === "ArrowDown"
        ? TABS[(at + 1) % TABS.length]
        : e.key === "ArrowLeft" || e.key === "ArrowUp"
          ? TABS[(at + TABS.length - 1) % TABS.length]
          : e.key === "Home"
            ? TABS[0]
            : e.key === "End"
              ? TABS[TABS.length - 1]
              : null;
    if (!next) return;
    e.preventDefault();
    setTab(next);
    buttons.current[next]?.focus();
  }

  return (
    <>
      <div className="lang-switch mac-tabs" role="tablist" aria-label={t.tabsLabel} onKeyDown={onKeyDown}>
        {TABS.map((id) => (
          <button
            key={id}
            ref={(el) => {
              buttons.current[id] = el;
            }}
            type="button"
            role="tab"
            id={`mac-tab-${id}`}
            aria-controls={`mac-panel-${id}`}
            aria-selected={tab === id}
            tabIndex={tab === id ? 0 : -1}
            onClick={() => setTab(id)}
          >
            {t.tabs[id].label}
          </button>
        ))}
      </div>
      <div className="mac-frame">
        <TrafficLights />
        {TABS.map((id) => (
          <div key={id} role="tabpanel" id={`mac-panel-${id}`} aria-labelledby={`mac-tab-${id}`} hidden={tab !== id}>
            <ThemedShot id={SCREEN[id]} light={t.tabs[id].light} dark={t.tabs[id].dark} sizes="(min-width: 1200px) 1120px, calc(100vw - 32px)" />
          </div>
        ))}
      </div>
      <p className="mac-caption">{t.tabs[tab].caption}</p>
    </>
  );
}
