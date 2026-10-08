// SPDX-License-Identifier: Apache-2.0

import { Icon } from "@/components/site/icons";
import { landing } from "@/content/landing";

const t = landing.listen;

export function ListenBand() {
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
        </div>
        <div className="listen">
          {t.items.map((item) => (
            <div className="listen-item" key={item.title}>
              <span className="listen-icon">
                <Icon name={item.icon} />
              </span>
              <h3>{item.title}</h3>
              <p>{item.body}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
