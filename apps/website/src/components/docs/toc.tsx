// SPDX-License-Identifier: Apache-2.0

import type { TOCItemType } from "fumadocs-core/toc";
import { useEffect, useState } from "react";
import { strings } from "@/content/strings";

/** h2/h3 of the page; the heading in view is marked `aria-current="true"`. Hidden below 1100 px by CSS. */
export function Toc({ items }: { items: TOCItemType[] }) {
  const entries = items.filter((i) => i.depth >= 2 && i.depth <= 3 && i.url.startsWith("#"));
  const [active, setActive] = useState<string>();
  const key = entries.map((e) => e.url).join(" ");

  useEffect(() => {
    const ids = key ? key.split(" ").map((u) => u.slice(1)) : [];
    const heads = ids.map((id) => document.getElementById(id)).filter((el): el is HTMLElement => el !== null);
    if (heads.length === 0) return;
    const obs = new IntersectionObserver(
      (list) => {
        const hit = list.find((en) => en.isIntersecting);
        if (hit) setActive(hit.target.id);
      },
      { rootMargin: "-80px 0px -70% 0px" },
    );
    for (const h of heads) obs.observe(h);
    return () => obs.disconnect();
  }, [key]);

  if (entries.length === 0) return null;
  return (
    <nav className="toc" aria-label={strings.docs.toc}>
      <p>{strings.docs.toc}</p>
      {entries.map((e) => (
        <a key={e.url} href={e.url} data-depth={e.depth} aria-current={active === e.url.slice(1) ? "true" : undefined}>
          {e.title}
        </a>
      ))}
    </nav>
  );
}
