// SPDX-License-Identifier: Apache-2.0

import { createFileRoute } from "@tanstack/react-router";
import { strings } from "@/content/strings";
import { pageHead } from "@/lib/seo";
import { docLink } from "@/lib/site-links";

export const Route = createFileRoute("/")({
  head: () => pageHead({ title: strings.site.title, description: strings.site.description, path: "/" }),
  component: Landing,
});

// Placeholder until the landing page lands (phase 3).
function Landing() {
  return (
    <main id="main" className="wrap hero">
      <h1>{strings.site.title}</h1>
      <p>
        <a href={docLink("privacy")}>{strings.nav.privacy}</a>
      </p>
    </main>
  );
}
