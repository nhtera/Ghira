// SPDX-License-Identifier: Apache-2.0

import { Icon } from "@/components/site/icons";
import { clock, COPIED, SAMPLE_LABELS } from "@/content/demo-data";
import { landing } from "@/content/landing";
import { DemoLanes } from "./demo-lanes";
import { DemoNotesPanel } from "./demo-notes-panel";
import { DemoTranscript } from "./demo-transcript";
import { useDemoClock } from "./hooks/use-demo-clock";
import { useMeetingLang } from "./hooks/use-meeting-lang";
import { LangSwitch } from "./lang-switch";

/**
 * The hero's app window: a sample meeting replayed live. The server draws it
 * at 27 s, in English; it plays only while on screen, can be paused, and with
 * reduced motion shows the finished meeting and no Pause button.
 */
export function LiveDemo() {
  const [lang] = useMeetingLang();
  const { ref, time, playing, reduced, toggle } = useDemoClock();
  const words = COPIED[lang];
  return (
    <>
      <div className={playing ? "window" : "window is-paused"} id="demo" role="group" aria-label={landing.demo.label} ref={ref}>
        <div className="win-bar">
          <div className="win-dots" aria-hidden="true">
            <span />
            <span />
            <span />
          </div>
          <span className="rec-state">
            <span className="rec-dot" aria-hidden="true" />
            <span>{words.recording}</span> <span className="mono">{clock(time)}</span>
          </span>
          <span className="win-title" lang={lang}>
            {SAMPLE_LABELS[lang].title}
          </span>
          <span className="privacy-pill">
            <Icon name="lock" />
            <span>{words.local}</span>
          </span>
        </div>
        <div className="win-body">
          <div className="win-main">
            <DemoLanes time={time} lang={lang} />
            <DemoTranscript time={time} lang={lang} />
          </div>
          <DemoNotesPanel time={time} lang={lang} />
        </div>
      </div>
      <div className="demo-foot">
        <span>{landing.demo.caption}</span>
        {reduced ? null : (
          <button className="link-btn" type="button" aria-pressed={!playing} onClick={toggle}>
            <Icon name={playing ? "pause" : "play"} />
            <span>{playing ? landing.demo.pause : landing.demo.play}</span>
          </button>
        )}
        <LangSwitch />
      </div>
    </>
  );
}
