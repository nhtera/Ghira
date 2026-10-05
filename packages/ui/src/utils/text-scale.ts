// SPDX-License-Identifier: Apache-2.0
// The phone's text scale (Dynamic Type stand-in): --ghi-text-scale drives the
// root font size, data-large-text lets big headings hyphenate (see ios.css).
export function setTextScale(scale: number) {
  const root = document.documentElement;
  root.style.setProperty("--ghi-text-scale", String(scale));
  if (scale > 1) root.dataset.largeText = "";
  else delete root.dataset.largeText;
}
