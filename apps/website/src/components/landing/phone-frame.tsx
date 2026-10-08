// SPDX-License-Identifier: Apache-2.0

import { type ScreenId, ThemedShot } from "./themed-shot";

/** An iPhone screenshot in a plain bezel, with its caption. */
export function PhoneFrame({ id, caption, light, dark }: { id: ScreenId; caption: string; light: string; dark: string }) {
  return (
    <figure className="phone">
      <div className="phone-body">
        <ThemedShot id={id} light={light} dark={dark} sizes="(min-width: 900px) 268px, 40vw" />
      </div>
      <figcaption>{caption}</figcaption>
    </figure>
  );
}
