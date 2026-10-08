// SPDX-License-Identifier: Apache-2.0

import { createFileRoute } from "@tanstack/react-router";
import { AfterCall } from "@/components/landing/after-call";
import { AppsBand } from "@/components/landing/apps-band";
import { CompareTable } from "@/components/landing/compare-table";
import { FaqSection } from "@/components/landing/faq";
import { FeatureGrid } from "@/components/landing/feature-grid";
import { GetBand } from "@/components/landing/get-band";
import { Hero } from "@/components/landing/hero";
import { ListenBand } from "@/components/landing/listen-band";
import { PrivacyBand } from "@/components/landing/privacy-band";
import { strings } from "@/content/strings";
import { pageHead } from "@/lib/seo";

export const Route = createFileRoute("/")({
  head: () => pageHead({ title: strings.site.title, description: strings.site.description, path: "/" }),
  component: Landing,
});

function Landing() {
  return (
    <main id="main">
      <Hero />
      <ListenBand />
      <AfterCall />
      <AppsBand />
      <PrivacyBand />
      <CompareTable />
      <FeatureGrid />
      <FaqSection />
      <GetBand />
    </main>
  );
}
