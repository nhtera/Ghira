// SPDX-License-Identifier: Apache-2.0

import { landing } from "@/content/landing";
import { MacShowcase } from "./mac-showcase";
import { PhoneFrame } from "./phone-frame";

const t = landing.apps;

export function AppsBand() {
  return (
    <section className="band" id={t.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{t.title}</h2>
          <p className="band-lead">{t.lead}</p>
        </div>
        <MacShowcase />
        <div className="phones">
          <div className="phones-copy">
            <h3>{t.phone.title}</h3>
            <p className="band-lead">{t.phone.lead}</p>
            <span className="tag">{t.phone.tag}</span>
          </div>
          <div className="phone-row">
            {t.phone.shots.map((shot) => (
              <PhoneFrame key={shot.id} id={shot.id} caption={shot.caption} light={shot.light} dark={shot.dark} />
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
