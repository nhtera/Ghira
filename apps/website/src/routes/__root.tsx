// SPDX-License-Identifier: Apache-2.0

import { createRootRoute, HeadContent, Outlet, Scripts } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { SiteFooter } from "@/components/site/site-footer";
import { SiteHeader } from "@/components/site/site-header";
import { strings } from "@/content/strings";
import { THEME_INIT_SCRIPT } from "@/lib/theme";
import { fontPreloads } from "@/styles/fonts";
import siteCss from "@/styles/site.css?url";

export const Route = createRootRoute({
  head: ({ matches }) => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" },
      { name: "color-scheme", content: "light dark" },
      // The not-found state (a docs link to a page that does not exist, on
      // client navigation) gets the 404 page's title and noindex.
      ...(matches.some((m) => m.status === "notFound" || ("globalNotFound" in m && m.globalNotFound))
        ? [{ title: strings.notFound.metaTitle }, { name: "robots", content: "noindex" }]
        : [{ title: strings.site.title }, { name: "description", content: strings.site.description }]),
    ],
    links: [...fontPreloads, { rel: "stylesheet", href: siteCss }, { rel: "icon", href: "/favicon.svg", type: "image/svg+xml" }],
    // Before first paint: the theme (stored pick, else the OS setting).
    scripts: [{ children: THEME_INIT_SCRIPT }],
  }),
  component: RootComponent,
});

function RootComponent() {
  return (
    <RootDocument>
      <Outlet />
    </RootDocument>
  );
}

function RootDocument({ children }: { children: ReactNode }) {
  return (
    // The init script sets data-theme before React hydrates.
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
      </head>
      <body>
        <SiteHeader />
        {children}
        <SiteFooter />
        <Scripts />
      </body>
    </html>
  );
}
