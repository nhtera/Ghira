// SPDX-License-Identifier: Apache-2.0

import { landing } from "@/content/landing";

const t = landing.features;

export function FeatureGrid() {
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
        </div>
        <div className="features">
          {t.items.map((item) => (
            <div className="feature" key={item.title}>
              <h3>{item.title}</h3>
              <p>{item.body}</p>
              {item.tag ? <span className="tag">{item.tag}</span> : null}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
