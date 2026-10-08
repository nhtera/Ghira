// SPDX-License-Identifier: Apache-2.0

import { createFileRoute } from "@tanstack/react-router";
import { strings } from "@/content/strings";
import { Icon } from "@/components/site/icons";

const t = strings.ogCard;

// The share card, 1200×630, rendered with the site's tokens and fonts and
// captured to public/og.png by `npm run og` (test/og-capture.mjs). Not linked
// from anywhere, not in the sitemap or llms.txt, and noindex.
export const Route = createFileRoute("/og-card")({
  head: () => ({ meta: [{ title: `${strings.site.name} share card` }, { name: "robots", content: "noindex" }] }),
  component: OgCard,
});

function OgCard() {
  return (
    <main id="main" className="og-card" data-og-card="">
      <div className="og-brand">
        <span className="brand-mark" aria-hidden="true">
          g
        </span>
        {strings.site.name}
      </div>
      <h1>{t.headline}</h1>
      <p className="og-sub">{t.sub}</p>
      <div className="og-window">
        <span className="privacy-pill">
          <Icon name="lock" />
          {t.local}
        </span>
        {t.lines.map((l) => (
          <div className="og-line" key={l.who}>
            <span className="avatar" style={{ background: `var(--s${l.slot})` }} aria-hidden="true">
              {l.who[0]}
            </span>
            <span className="og-who" style={{ color: `var(--s${l.slot})` }}>
              {l.who}
            </span>
            <span className="og-text">{l.text}</span>
          </div>
        ))}
      </div>
    </main>
  );
}
