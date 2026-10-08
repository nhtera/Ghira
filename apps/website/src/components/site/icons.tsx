// SPDX-License-Identifier: Apache-2.0

// The prototype's icon set, as inline SVG (no icon font, no network).
// Decorative: every icon is aria-hidden; the text next to it is the label.

import type { ReactNode } from "react";

const ICONS = {
  lock: { box: 16, body: <path fill="currentColor" d="M5 7V5a3 3 0 1 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v5a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 3 13.5v-5A1.5 1.5 0 0 1 4.5 7H5Zm1.5 0h3V5a1.5 1.5 0 0 0-3 0v2Z" /> },
  moon: { box: 20, body: <path fill="currentColor" d="M8.2 2.6a7.5 7.5 0 1 0 9.2 9.2A6 6 0 0 1 8.2 2.6Z" /> },
  sun: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
        <circle cx="10" cy="10" r="3.6" />
        <path d="M10 1.8v2M10 16.2v2M1.8 10h2M16.2 10h2M4.2 4.2l1.4 1.4M14.4 14.4l1.4 1.4M4.2 15.8l1.4-1.4M14.4 5.6l1.4-1.4" />
      </g>
    ),
  },
  pause: { box: 16, body: <path fill="currentColor" d="M4 3h3v10H4zM9 3h3v10H9z" /> },
  play: { box: 16, body: <path fill="currentColor" d="M4.5 2.8v10.4L13 8z" /> },
  nobot: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
        <circle cx="10" cy="7" r="3" />
        <path d="M4.5 16.5c.8-2.6 3-4 5.5-4s4.7 1.4 5.5 4M3 3l14 14" />
      </g>
    ),
  },
  code: { box: 20, body: <path fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" d="m7 6-4 4 4 4M13 6l4 4-4 4" /> },
  user: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.6">
        <circle cx="10" cy="10" r="7.5" />
        <circle cx="10" cy="8.3" r="2.5" />
        <path d="M5.3 15.6c1-1.7 2.7-2.6 4.7-2.6s3.7.9 4.7 2.6" />
      </g>
    ),
  },
  apps: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.6">
        <rect x="2.5" y="4" width="15" height="10" rx="1.5" />
        <path d="M7 17h6M10 14v3" strokeLinecap="round" />
      </g>
    ),
  },
  room: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
        <circle cx="5" cy="7" r="2" />
        <circle cx="15" cy="7" r="2" />
        <circle cx="10" cy="5.5" r="2" />
        <path d="M1.8 14c.5-2 1.7-3 3.2-3s2.7 1 3.2 3M11.8 14c.5-2 1.7-3 3.2-3s2.7 1 3.2 3M6.8 12.5c.5-2 1.7-3 3.2-3s2.7 1 3.2 3" />
      </g>
    ),
  },
  yes: { box: 16, body: <path fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" d="m3 8.5 3.2 3L13 4.5" /> },
  no: { box: 16, body: <path fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" d="M4 4l8 8M12 4l-8 8" /> },
  search: {
    box: 20,
    body: (
      <g fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
        <circle cx="8.5" cy="8.5" r="5.5" />
        <path d="m13 13 4.5 4.5" />
      </g>
    ),
  },
} satisfies Record<string, { box: number; body: ReactNode }>;

export type IconName = keyof typeof ICONS;

export function Icon({ name, className }: { name: IconName; className?: string }) {
  const icon = ICONS[name];
  return (
    <svg className={className} viewBox={`0 0 ${icon.box} ${icon.box}`} aria-hidden="true" focusable="false">
      {icon.body}
    </svg>
  );
}
