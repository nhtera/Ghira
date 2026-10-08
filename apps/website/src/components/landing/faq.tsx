// SPDX-License-Identifier: Apache-2.0

import { FAQ, FAQ_TITLE } from "@/content/faq";
import { landing } from "@/content/landing";

/** The questions as <details>: every answer is in the HTML, open or not. */
export function FaqSection() {
  return (
    <section className="band" id={landing.faq.id}>
      <div className="wrap">
        <div className="band-head">
          <h2>{FAQ_TITLE}</h2>
        </div>
        <div className="faq">
          {FAQ.map((item) => (
            <details key={item.q}>
              <summary>{item.q}</summary>
              <p>{item.a}</p>
            </details>
          ))}
        </div>
      </div>
    </section>
  );
}
