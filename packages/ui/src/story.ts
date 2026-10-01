// SPDX-License-Identifier: Apache-2.0
// Story format for the gallery (CSF-like, so moving to Storybook/Ladle later
// is mechanical). A `*.stories.tsx` file default-exports `StoryMeta` and
// exports one `Story` per state:
//
//   export default { title: "Speaker chip" } satisfies StoryMeta;
//   export const Unknown: Story = { render: () => <SpeakerChip ... /> };
import type { ReactNode } from "react";

export type StoryMeta = {
  title: string;
  /** Width of each state's frame in the contact sheet (px). */
  width?: number;
};

export type Story = {
  render: () => ReactNode;
  /** Shown under the state in the gallery. */
  note?: string;
  /**
   * Renders a modal/floating layer (dialog, sheet, open menu). The contact
   * sheet links to it instead of rendering it inline; `?state=` shows it.
   */
  overlay?: boolean;
};
