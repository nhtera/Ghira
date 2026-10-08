// SPDX-License-Identifier: Apache-2.0

import { Link } from "@tanstack/react-router";
import { strings } from "@/content/strings";

/** The app icon's serif "g" on the accent square, and the name. Links home. */
export function Brand() {
  return (
    <Link className="brand" to="/" aria-label={strings.nav.home}>
      <span className="brand-mark" aria-hidden="true">
        g
      </span>
      {strings.site.name}
    </Link>
  );
}
