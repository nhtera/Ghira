// SPDX-License-Identifier: Apache-2.0

import { landing } from "@/content/landing";
import { RedactionPanes } from "./redaction-panes";

const t = landing.privacy;

export function PrivacyBand() {
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
        </div>
        <div className="rules">
          {t.rules.map((rule) => (
            <div className="rule" key={rule.what}>
              <p className="rule-what">
                {rule.what}
                {rule.small ? <small>{rule.small}</small> : null}
              </p>
              <p className="rule-how">
                <span className={`verdict v-${rule.tone}`}>{rule.verdict}</span>
                {rule.how}
                {rule.strong ? (
                  <>
                    {" "}
                    <strong>{rule.strong}</strong> {rule.after}
                  </>
                ) : null}
              </p>
            </div>
          ))}
        </div>
        <div className="cloud">
          <h3>{t.cloud.title}</h3>
          <p className="band-lead">{t.cloud.lead}</p>
          <RedactionPanes />
          <p className="footnote">{t.cloud.footnote}</p>
        </div>
      </div>
    </section>
  );
}
