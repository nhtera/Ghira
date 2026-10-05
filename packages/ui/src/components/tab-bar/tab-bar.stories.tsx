// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import type { Story, StoryMeta } from "../../story";
import { useMobileT } from "../../utils/mobile-t";
import { TabBar } from "./tab-bar";

export default { title: "Tab bar (iOS)", platform: "ios" } satisfies StoryMeta;

function Demo({ initial }: { initial: string }) {
  const t = useMobileT();
  const [value, setValue] = useState(initial);
  return (
    <TabBar
      label={t("mobile.shell.tabs")}
      value={value}
      onChange={setValue}
      items={[
        { id: "meetings", label: t("mobile.tabs.meetings"), icon: "event_note" },
        { id: "record", label: t("mobile.tabs.record"), icon: "radio_button_checked", emphasized: true },
        { id: "search", label: t("mobile.tabs.search"), icon: "search" },
        { id: "settings", label: t("mobile.tabs.settings"), icon: "settings" },
      ]}
    />
  );
}

export const MeetingsSelected: Story = { render: () => <Demo initial="meetings" /> };
export const RecordSelected: Story = { render: () => <Demo initial="record" />, note: "Record shares the line of the other tabs and turns red when selected." };
export const SettingsSelected: Story = { render: () => <Demo initial="settings" /> };
