// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { EmptyState } from "./empty-state";

export default { title: "Empty state", width: 360 } satisfies StoryMeta;

const noop = () => {};

export const Library: Story = { render: () => <EmptyState kind="library" onPrimary={noop} onSecondary={noop} /> };
export const SearchNoResults: Story = { render: () => <EmptyState kind="search" query="chot deadline" /> };
export const People: Story = { render: () => <EmptyState kind="people" /> };
export const Ask: Story = { render: () => <EmptyState kind="ask" /> };
export const ImportQueue: Story = { render: () => <EmptyState kind="import" onPrimary={noop} /> };
