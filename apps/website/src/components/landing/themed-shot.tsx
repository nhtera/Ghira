// SPDX-License-Identifier: Apache-2.0

import screens from "../../content/screens.json";

export type ScreenId = keyof typeof screens;

interface Candidate {
  w: number;
  src: string;
}

const srcset = (list: Candidate[]) => list.map((c) => `${c.src} ${c.w}w`).join(", ");
const largest = (list: Candidate[]) => list.reduce((a, b) => (b.w > a.w ? b : a)).src;

/**
 * A real screenshot in both themes: two images, and the theme's CSS
 * (--show-light / --show-dark) shows one. Width and height come from the
 * manifest so the page does not shift while the image loads; the hidden
 * image is never fetched (lazy).
 */
export function ThemedShot({ id, light, dark, sizes }: { id: ScreenId; light: string; dark: string; sizes: string }) {
  const shot = screens[id];
  return (
    <>
      <img
        className="shot-light"
        src={largest(shot.light)}
        srcSet={srcset(shot.light)}
        sizes={sizes}
        width={shot.width}
        height={shot.height}
        alt={light}
        loading="lazy"
        decoding="async"
      />
      <img
        className="shot-dark"
        src={largest(shot.dark)}
        srcSet={srcset(shot.dark)}
        sizes={sizes}
        width={shot.width}
        height={shot.height}
        alt={dark}
        loading="lazy"
        decoding="async"
      />
    </>
  );
}
