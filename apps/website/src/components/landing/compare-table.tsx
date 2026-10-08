// SPDX-License-Identifier: Apache-2.0

import { Icon } from "@/components/site/icons";
import { landing } from "@/content/landing";

const t = landing.compare;

/**
 * Generic columns ("Typical cloud note taker" vs Ghira); no product is named
 * in the table. Each row's claim is backed by public documentation of
 * well-known tools, linked in the sources list under the table. Below 640 px
 * every row stacks into one block (the cells carry data-label).
 */
export function CompareTable() {
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
        </div>
        <div className="compare-scroll">
          <table className="compare">
            <thead>
              <tr>
                <th scope="col">
                  <span className="sr-only">{t.question}</span>
                </th>
                <th scope="col">{t.cloud}</th>
                <th scope="col" className="ours">
                  {t.ours}
                </th>
              </tr>
            </thead>
            <tbody>
              {t.rows.map((row) => (
                <tr key={row.q}>
                  <th scope="row">{row.q}</th>
                  <td data-label={t.cloudShort}>
                    {row.cloudMark === "varies" ? (
                      <span className="mark varies">{row.cloud}</span>
                    ) : (
                      <span className="mark no">
                        <Icon name="no" />
                        {row.cloud}
                      </span>
                    )}
                  </td>
                  <td className="ours" data-label={t.ours}>
                    <span className="mark yes">
                      <Icon name="yes" />
                      {row.ours}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="compare-sources footnote">
          <p>
            <strong>{t.sourcesTitle}.</strong> {t.sourcesLead}
          </p>
          <ul className="source-list">
            {t.rows.map((row) => (
              <li key={row.q}>
                <span>{row.q}: </span>
                {row.sources.map((s, i) => (
                  <span key={s.href + s.label}>
                    {i > 0 ? ", " : null}
                    <a href={s.href}>{s.label}</a>
                  </span>
                ))}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </section>
  );
}
