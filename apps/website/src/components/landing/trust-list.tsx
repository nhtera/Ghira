// SPDX-License-Identifier: Apache-2.0

import { Icon } from "@/components/site/icons";
import { landing } from "@/content/landing";

export function TrustList() {
  return (
    <ul className="trust" aria-label={landing.hero.trustLabel}>
      {landing.hero.trust.map((item) => (
        <li key={item.label}>
          <Icon name={item.icon} />
          {item.label}
        </li>
      ))}
    </ul>
  );
}
