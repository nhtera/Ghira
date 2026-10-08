// SPDX-License-Identifier: Apache-2.0

import { useRef } from "react";
import quickstart from "../../../content/generated/quickstart.json" with { type: "json" };
import { landing } from "@/content/landing";
import { docLink } from "@/lib/site-links";
import { CopyButton } from "./copy-button";

const t = landing.get;
const COMMANDS: string = quickstart.commands;

/** Where Ghira runs today, and the build-from-source commands (one source: the README's quick start). */
export function GetBand() {
  const pre = useRef<HTMLPreElement>(null);
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
        </div>
        <div className="get">
          <div className="platforms" aria-label={t.platformsLabel}>
            {t.platforms.map((p) => (
              <div className="platform" key={p.name}>
                <b>{p.name}</b>
                <small>{p.note}</small>
                <span className={`pill pill-${p.tone}`}>{p.pill}</span>
              </div>
            ))}
          </div>
          <div>
            <div className="code">
              <div className="code-head">
                <span>{t.codeTitle}</span>
                <CopyButton text={`${t.codeComment}\n${COMMANDS}`} target={pre} />
              </div>
              <pre ref={pre}>
                <span className="c">{t.codeComment}</span>
                {"\n"}
                {COMMANDS}
              </pre>
            </div>
            <p className="get-note">
              {t.noteBefore}
              <a href={docLink("install")}>{t.noteLink}</a>
              {t.noteAfter}
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
