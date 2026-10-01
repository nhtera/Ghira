// SPDX-License-Identifier: Apache-2.0
// Gallery: every component state, addressable by URL so Playwright can visit
// each one (axe, CSP, visual snapshots):
//   ?story=<id>[&state=<export>]&theme=light|dark&lang=en|vi&platform=mac|win
// Without `story`, an index. Without `state`, every state of the story.
import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { I18nextProvider } from "react-i18next";
import { initI18n, type Locale } from "@ghi/i18n";
import "../src/styles.css";
import { PlatformProvider, type Platform } from "../src/platform/platform";
import { TooltipProvider } from "../src/primitives/tooltip";
import { ToastProvider } from "../src/primitives/toast";
import { applyTheme } from "../src/theme/theme";
import type { Story, StoryMeta } from "../src/story";

type Module = { default: StoryMeta } & Record<string, Story | StoryMeta>;
const modules = import.meta.glob<Module>("../src/**/*.stories.tsx", { eager: true });

export type GalleryEntry = { id: string; title: string; states: string[] };

const entries = Object.entries(modules)
  .map(([path, mod]) => {
    const id = path.split("/").pop()!.replace(".stories.tsx", "");
    const states = Object.keys(mod).filter((k) => k !== "default");
    return { id, title: mod.default.title, states, mod };
  })
  .sort((a, b) => a.title.localeCompare(b.title));

// For Playwright: the list comes from the glob, never from a hand-kept list.
(window as unknown as { __GALLERY__: GalleryEntry[] }).__GALLERY__ = entries.map(({ id, title, states }) => ({ id, title, states }));

const q = new URLSearchParams(location.search);
const lang = (q.get("lang") === "vi" ? "vi" : "en") as Locale;
const platform = (q.get("platform") === "win" ? "win" : "mac") as Platform;
applyTheme(q.get("theme") === "dark" ? "dark" : "light");
document.documentElement.lang = lang;
document.documentElement.dataset.platform = platform;
const i18n = initI18n(lang);

function Frame({ label, width, note, children }: { label: string; width?: number; note?: string; children: ReactNode }) {
  return (
    <section data-story-state={label} style={{ width }} className="flex flex-col gap-2">
      <h2 className="text-label m-0 text-muted">{label.replace(/([a-z])([A-Z])/g, "$1 $2")}</h2>
      <div className="rounded-panel border border-line bg-surface p-4">{children}</div>
      {note && <p className="text-small m-0 text-muted">{note}</p>}
    </section>
  );
}

function Page() {
  const id = q.get("story");
  const entry = entries.find((e) => e.id === id);
  if (!entry) {
    return (
      <main className="p-7">
        <h1 className="text-title">Components</h1>
        <ul>
          {entries.map((e) => (
            <li key={e.id}>
              <a className="text-accent" href={`?story=${e.id}`}>
                {e.title}
              </a>
            </li>
          ))}
        </ul>
      </main>
    );
  }
  const only = q.get("state");
  const states = only ? entry.states.filter((s) => s === only) : entry.states;
  return (
    <main className="p-7" data-story={entry.id}>
      <h1 className="text-title mt-0">{entry.title}</h1>
      <div className="flex flex-wrap items-start gap-5">
        {states.map((s) => {
          const story = entry.mod[s] as Story;
          return (
            <Frame key={s} label={s} width={entry.mod.default.width} note={story.note}>
              {story.overlay && !only ? (
                <a className="text-accent" href={`?${new URLSearchParams({ ...Object.fromEntries(q), state: s })}`}>
                  Open “{s}”
                </a>
              ) : (
                story.render()
              )}
            </Frame>
          );
        })}
      </div>
    </main>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <I18nextProvider i18n={i18n}>
      <PlatformProvider value={platform}>
        <TooltipProvider>
          <ToastProvider label="Notifications">
            <Page />
          </ToastProvider>
        </TooltipProvider>
      </PlatformProvider>
    </I18nextProvider>
  </StrictMode>,
);
