// SPDX-License-Identifier: Apache-2.0

import { Link } from "@tanstack/react-router";
import { landing } from "@/content/landing";
import { REPO_URL } from "@/lib/urls";
import { LiveDemo } from "./live-demo";
import { TrustList } from "./trust-list";

const t = landing.hero;

export function Hero() {
  return (
    <section className="hero" id="top">
      <div className="wrap">
        <div className="hero-copy">
          <a className="announce" href="#apps">
            <span className="announce-tag">{t.announceTag}</span>
            {t.announce}
          </a>
          <h1>{t.title}</h1>
          <p className="hero-sub">{t.sub}</p>
          <div className="hero-actions">
            <a className="btn btn-primary" href={REPO_URL}>
              {t.primary}
            </a>
            <Link className="btn" to="/docs/$" params={{ _splat: "" }}>
              {t.secondary}
            </Link>
          </div>
          <TrustList />
          <p className="hero-status">{t.status}</p>
        </div>
        <LiveDemo />
      </div>
    </section>
  );
}
