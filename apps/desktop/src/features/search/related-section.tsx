// SPDX-License-Identifier: Apache-2.0
// Library search: passages related by meaning, after the keyword hits.
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { formatClock, formatDate, type Locale } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import type { RelatedHit } from "../../bindings";
import type { OpenHit } from "./search-results";

export function RelatedSection({ hits, exclude, onOpen }: { hits: RelatedHit[]; exclude: ReadonlySet<string>; onOpen: (h: OpenHit) => void }) {
  const { t, i18n } = useTranslation();
  const locale = (i18n.language === "vi" ? "vi" : "en") as Locale;
  const headingId = useId();
  const shown = hits.filter((h) => !exclude.has(h.meeting.meeting));
  if (shown.length === 0) return null;
  return (
    <section aria-labelledby={headingId} data-testid="related-section" className="mt-4">
      <h2 id={headingId} className="text-small m-0 mb-2 flex items-center gap-1.5 font-semibold text-muted">
        <Icon name="auto_awesome" size={16} />
        {t("ask.related.title")}
      </h2>
      <ul className="m-0 flex list-none flex-col gap-2 p-0">
        {shown.map((h) => (
          <li key={h.meeting.meeting}>
            <button
              type="button"
              onClick={() => onOpen({ meeting: h.meeting.meeting, tab: h.t0Ms != null ? "transcript" : "notes", tMs: h.t0Ms })}
              className="flex w-full flex-col gap-1 rounded-row border border-line bg-surface px-3 py-2 text-left hover:bg-surface2"
            >
              <span className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold">{h.meeting.title}</span>
                {h.t0Ms != null && <span className="text-mono flex-none text-muted">{formatClock(h.t0Ms)}</span>}
                {h.meeting.startedAt != null && <span className="text-small flex-none text-muted">{formatDate(h.meeting.startedAt, locale)}</span>}
              </span>
              <span className="font-serif text-[14.5px] text-muted [overflow-wrap:anywhere]">{h.quote}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
