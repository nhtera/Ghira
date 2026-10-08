// SPDX-License-Identifier: Apache-2.0

import { Link, useRouterState } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { strings } from "@/content/strings";
import { REPO_URL } from "@/lib/urls";
import { Brand } from "./brand";
import { ThemeToggle } from "./theme-toggle";

const t = strings.nav;

/**
 * Sticky header. It gains a shadow once the page has scrolled: a sentinel
 * at the top of the page leaves the viewport (no scroll listener).
 */
export function SiteHeader() {
  const sentinel = useRef<HTMLDivElement>(null);
  const [scrolled, setScrolled] = useState(false);
  const inDocs = useRouterState({ select: (s) => s.location.pathname.startsWith("/docs") });
  useEffect(() => {
    const el = sentinel.current;
    if (!el) return;
    const obs = new IntersectionObserver(([e]) => setScrolled(!e.isIntersecting));
    obs.observe(el);
    return () => obs.disconnect();
  }, []);
  return (
    <>
      <a className="skip" href="#main">
        {t.skip}
      </a>
      <div className="scroll-sentinel" ref={sentinel} aria-hidden="true" />
      <header className={scrolled ? "site-head is-scrolled" : "site-head"}>
        <div className="wrap">
          <Brand />
          <nav className="head-nav" aria-label={t.label}>
            <Link to="/docs/$" params={{ _splat: "" }} aria-current={inDocs ? "page" : undefined}>
              {t.docs}
            </Link>
            <Link className="hide-sm" to="/" hash="privacy">
              {t.privacy}
            </Link>
            <Link className="hide-sm" to="/" hash="faq">
              {t.faq}
            </Link>
            <a className="hide-xs" href={REPO_URL}>
              {t.github}
            </a>
            <ThemeToggle />
            <Link className="head-cta" to="/" hash="get">
              {t.cta}
            </Link>
          </nav>
        </div>
      </header>
    </>
  );
}
