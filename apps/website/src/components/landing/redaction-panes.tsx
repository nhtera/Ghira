// SPDX-License-Identifier: Apache-2.0

import type { ReactNode } from "react";
import { landing } from "@/content/landing";

const t = landing.privacy.cloud;

function Pane({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <div className="pane">
      <div className="pane-head">
        {title}
        {note ? <span>{note}</span> : null}
      </div>
      <div className="pane-body">{children}</div>
    </div>
  );
}

/** Before and after hiding names: what stays on the Mac, and what the cloud provider would receive. */
export function RedactionPanes() {
  return (
    <div className="cloud-panes">
      <Pane title={t.onMac}>
        {t.before.map((line, i) => (
          <p key={i}>
            {line.map((part, k) => (typeof part === "string" ? part : <span key={k} className="pii">{part.pii}</span>))}
          </p>
        ))}
      </Pane>
      <Pane title={t.sent} note={t.sentNote}>
        {t.after.map((line, i) => (
          <p key={i}>
            {line.map((part, k) => (typeof part === "string" ? part : <span key={k} className="mask">{part.mask}</span>))}
          </p>
        ))}
      </Pane>
    </div>
  );
}
