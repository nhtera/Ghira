// SPDX-License-Identifier: Apache-2.0

// The site's shared copy (header, footer, 404, docs chrome). English only.
// The landing page's copy is in landing.ts. Components never hold copy.

export const strings = {
  site: {
    name: "Ghira",
    title: "Ghira: meeting notes that stay on your Mac",
    description:
      "Ghira records calls and in-person meetings in English and Vietnamese, shows who said what as people talk, and writes notes you can check line by line. Everything runs on your Mac. Open source.",
  },
  llms: {
    summary:
      "Ghira is an offline-first AI meeting note taker for English and Vietnamese. It records calls and in-person meetings on a Mac, tells speakers apart as they talk, and writes notes with citations to the transcript. Audio, transcripts and notes stay on the device; cloud AI is an opt-in per meeting and sends reviewed transcript text only.",
    details: [
      "Status: pre-release. The Mac app (Apple Silicon, macOS 14.2 or later) is built from source; the iPhone app is in testing. Open source under the Apache 2.0 license: https://github.com/nhtera/Ghira",
    ],
  },
  ogCard: {
    headline: "Meeting notes that stay on your Mac.",
    sub: "English and Vietnamese. Speakers told apart as they talk. Open source.",
    lines: [
      { who: "Linh", slot: 2, text: "I collected feedback from 12 user tests." },
      { who: "Minh", slot: 4, text: "Bản Nemotron mới giảm lỗi nhiều." },
      { who: "Sarah", slot: 8, text: "People want to rename speakers during the call." },
    ],
    local: "Local only",
  },
  nav: {
    skip: "Skip to content",
    label: "Site",
    home: "Ghira home",
    docs: "Docs",
    privacy: "Privacy",
    faq: "FAQ",
    github: "GitHub",
    cta: "Get Ghira",
  },
  theme: {
    toDark: "Switch to dark theme",
    toLight: "Switch to light theme",
  },
  footer: {
    ctaTitle: "Your next meeting can stay on your Mac.",
    primary: "Get the source on GitHub",
    secondary: "Read the docs",
    blurb: "Offline meeting notes for English and Vietnamese, with speakers told apart as they talk. Free and open source.",
    quiet: "This website has no cookies and no analytics.",
    product: {
      title: "Product",
      links: [
        { label: "How it listens", hash: "listen" },
        { label: "Notes", hash: "after" },
        { label: "Mac and iPhone", hash: "apps" },
        { label: "Privacy", hash: "privacy" },
        { label: "FAQ", hash: "faq" },
      ],
    },
    docs: {
      title: "Docs",
      label: "Documentation",
      links: [
        { label: "Install from source", slug: "install" },
        { label: "Record your first meeting", slug: "getting-started" },
        { label: "Command line", slug: "cli" },
        { label: "Release notes", slug: "release-notes/0-1-0-alpha-1" },
      ],
    },
    project: {
      title: "Project",
      github: "GitHub",
      links: [
        { label: "Contributing", slug: "contributing", file: "CONTRIBUTING.md" },
        { label: "Security", slug: "security", file: "SECURITY.md" },
        { label: "Trademarks", slug: "trademarks", file: "TRADEMARKS.md" },
        { label: "Code of conduct", file: "CODE_OF_CONDUCT.md" },
      ],
    },
    copyright: "Copyright 2026 The Ghira Authors. Licensed under Apache 2.0.",
    status: "Ghira is pre-release software.",
  },
  notFound: {
    metaTitle: "Page not found · Ghira",
    code: "404",
    title: "This page isn't here",
    body: "The link may be old, or the page moved. The docs and the home page are a good place to start.",
    home: "Go to the home page",
    docs: "Read the docs",
  },
  docs: {
    titleSuffix: " · Ghira docs",
    navLabel: "Docs",
    menu: "Docs menu",
    search: "Search docs",
    searchShortcut: "⌘K",
    searchEmpty: (q: string) => `No page matches “${q}”. Try a shorter word.`,
    searchLoading: "Loading…",
    searchError: "Search could not load. Try again.",
    toc: "On this page",
    pager: "Previous and next page",
    previous: "Previous",
    next: "Next",
    edit: "Edit this page on GitHub",
    lastUpdated: "Last updated",
    copy: "Copy",
    copied: "Copied",
    selected: "Selected",
    codeLabel: "Code",
    tableLabel: "Table",
    indexSections: "Sections",
  },
} as const;
