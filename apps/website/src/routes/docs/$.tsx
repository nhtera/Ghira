// SPDX-License-Identifier: Apache-2.0

import { createFileRoute } from "@tanstack/react-router";
import browserCollections from "collections/browser";
import { Suspense } from "react";
import { getMDXComponents } from "@/components/mdx";
import { strings } from "@/content/strings";
import { loadDocsPage } from "@/lib/docs-data";
import { pageHead } from "@/lib/seo";
import { doc } from "@/lib/urls";

export const Route = createFileRoute("/docs/$")({
  component: Page,
  loader: async ({ params }) => {
    const slugs = params._splat?.split("/").filter(Boolean) ?? [];
    const data = await loadDocsPage(slugs);
    await clientLoader.preload(data.path);
    return { ...data, slug: slugs.join("/") };
  },
  head: ({ loaderData }) =>
    loaderData
      ? pageHead({
          title: loaderData.slug ? `${loaderData.title}${strings.docs.titleSuffix}` : `${loaderData.title} · Ghira`,
          description: loaderData.description,
          path: doc(loaderData.slug),
        })
      : {},
});

const clientLoader = browserCollections.docs.createClientLoader({
  component({ frontmatter, default: MDX }) {
    return (
      <article className="article" data-docs-article="">
        <h1>{frontmatter.title}</h1>
        <p className="lead">{frontmatter.description}</p>
        <MDX components={getMDXComponents()} />
      </article>
    );
  },
});

// Placeholder chrome until the docs site lands (phase 6).
function Page() {
  const { path } = Route.useLoaderData();
  return (
    <main id="main" className="wrap docs">
      <Suspense>{clientLoader.useContent(path)}</Suspense>
    </main>
  );
}
