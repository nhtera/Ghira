// SPDX-License-Identifier: Apache-2.0

import { Link } from "@tanstack/react-router";
import { strings } from "@/content/strings";
import { docLink, rootDocLink } from "@/lib/site-links";
import { REPO_URL } from "@/lib/urls";
import { Brand } from "./brand";
import { Icon } from "./icons";

const t = strings.footer;

/** The prototype's footer: a closing call to action, then grouped links. */
export function SiteFooter() {
  return (
    <footer className="site-foot">
      <div className="wrap foot-cta">
        <h2>{t.ctaTitle}</h2>
        <div className="hero-actions">
          <a className="btn btn-primary" href={REPO_URL}>
            {t.primary}
          </a>
          <Link className="btn" to="/docs/$" params={{ _splat: "" }}>
            {t.secondary}
          </Link>
        </div>
      </div>
      <div className="wrap foot-grid">
        <div className="foot-brand">
          <Brand />
          <p>{t.blurb}</p>
          <p className="foot-quiet">
            <Icon name="lock" />
            {t.quiet}
          </p>
        </div>
        <nav className="foot-col" aria-label={t.product.title}>
          <p>{t.product.title}</p>
          {t.product.links.map((l) => (
            <a key={l.hash} href={`/#${l.hash}`}>
              {l.label}
            </a>
          ))}
        </nav>
        <nav className="foot-col" aria-label={t.docs.label}>
          <p>{t.docs.title}</p>
          {t.docs.links.map((l) => (
            <a key={l.slug} href={docLink(l.slug)}>
              {l.label}
            </a>
          ))}
        </nav>
        <nav className="foot-col" aria-label={t.project.title}>
          <p>{t.project.title}</p>
          <a href={REPO_URL}>{t.project.github}</a>
          {t.project.links.map((l) => (
            <a key={l.file} href={rootDocLink("slug" in l ? l.slug : undefined, l.file)}>
              {l.label}
            </a>
          ))}
        </nav>
      </div>
      <div className="wrap foot-bottom">
        <span>{t.copyright}</span>
        <span>{t.status}</span>
      </div>
    </footer>
  );
}
