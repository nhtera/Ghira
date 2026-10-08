// SPDX-License-Identifier: Apache-2.0

import { useRef, useState } from "react";
import { Segmented } from "@/components/site/segmented";
import { landing } from "@/content/landing";
import { useMeetingLang } from "./hooks/use-meeting-lang";
import { NARROW, REDUCED_MOTION, useMediaQuery } from "./hooks/use-media-query";
import { LangSwitch } from "./lang-switch";
import { type ActiveCite, DEFAULT_CITE, NotesView } from "./notes-view";
import { TranscriptPanel } from "./transcript-panel";

type Mode = "typed" | "full";
const t = landing.after;

/**
 * "After the call": the notes with a time chip on every sentence, beside the
 * transcript. A chip highlights its line and scrolls the panel to it; on
 * narrow screens (the panel is far below) it shows the line under the note.
 */
export function AfterCall() {
  const [lang] = useMeetingLang();
  const [mode, setMode] = useState<Mode>("full");
  const [active, setActive] = useState<ActiveCite>(DEFAULT_CITE);
  // The quote opens on a press, and goes away when the language or view changes.
  const [opened, setOpened] = useState<{ lang: string; mode: Mode; id: string } | null>(null);
  const narrow = useMediaQuery(NARROW);
  const reduced = useMediaQuery(REDUCED_MOTION);
  const scroller = useRef<HTMLDivElement>(null);

  const quote = narrow && opened !== null && opened.lang === lang && opened.mode === mode && opened.id === active.id;

  function onCite(cite: ActiveCite) {
    setActive(cite);
    if (narrow) {
      setOpened({ lang, mode, id: cite.id });
      return;
    }
    const box = scroller.current;
    const line = box?.querySelector(`.line[data-i="${cite.lines[0]}"]`);
    if (!box || !line) return;
    const top = line.getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop;
    box.scrollTo({ top: top - box.clientHeight / 3, behavior: reduced ? "auto" : "smooth" });
  }

  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
          <LangSwitch />
        </div>
        <div className="after">
          <div className="notes-col">
            <Segmented
              className="notes-mode"
              label={t.modeLabel}
              value={mode}
              onChange={setMode}
              options={[
                { value: "typed", label: t.typed },
                { value: "full", label: t.full },
              ]}
            />
            <NotesView mode={mode} lang={lang} active={active} quote={quote} onCite={onCite} />
          </div>
          <TranscriptPanel lang={lang} cited={mode === "full" ? active.lines : []} scrollerRef={scroller} />
        </div>
      </div>
    </section>
  );
}
