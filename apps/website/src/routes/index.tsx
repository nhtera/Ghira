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
import { FAQ } from "@/content/faq";
import { strings } from "@/content/strings";
import { faqPage, jsonForScript, softwareApplication } from "@/lib/json-ld";
import { pageHead } from "@/lib/seo";

// Structured data: an inline script, hashed into this page's CSP at build time.
const JSON_LD = jsonForScript([softwareApplication({ name: strings.site.name, description: strings.site.description }), faqPage(FAQ)]);

export const Route = createFileRoute("/")({
  head: () => ({
    ...pageHead({ title: strings.site.title, description: strings.site.description, path: "/" }),
    scripts: [{ type: "application/ld+json", children: JSON_LD }],
  }),
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
