// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { Kbd } from "./kbd";

export default { title: "Keycap chip", width: 360 } satisfies StoryMeta;

export const Mac: Story = { render: () => <Kbd shortcut="⌘⇧R" /> };
export const Windows: Story = { render: () => <Kbd shortcut="Ctrl+Shift+R" /> };
export const Large: Story = { render: () => <Kbd size="lg" shortcut="⌘⇧R" /> };
