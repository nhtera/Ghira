// SPDX-License-Identifier: Apache-2.0
import * as Switch from "@radix-ui/react-switch";
import type { Story, StoryMeta } from "../../story";
import { useMobileT } from "../../utils/mobile-t";
import { ListRow, ListSection } from "./list";

export default { title: "List (iOS)", platform: "ios" } satisfies StoryMeta;

export const Grouped: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <>
        <ListSection header={t("mobile.tabs.meetings")} footer={t("mobile.record.pocket.hint")}>
          <ListRow icon="mic" title={t("mobile.record.title")} subtitle={t("mobile.record.onPhone")} chevron onPress={() => {}} />
          <ListRow icon="language" title={t("mobile.import.lang.auto")} value={t("mobile.import.lang.both")} chevron onPress={() => {}} />
          <ListRow icon="desktop_windows" title={t("mobile.target.desktop")} value={t("mobile.chip.waitingForWifi")} />
        </ListSection>
        <ListSection>
          <ListRow title={t("mobile.common.replay")} onPress={() => {}} />
          <ListRow title={t("mobile.common.skip")} disabled />
          <ListRow title={t("mobile.record.stopAndSave")} destructive onPress={() => {}} />
        </ListSection>
      </>
    );
  },
};

export const LongText: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <ListSection header={t("mobile.detail.notes")}>
        <ListRow
          icon="record_voice_over"
          title={t("mobile.onboarding.languages.title")}
          subtitle={t("mobile.onboarding.languages.subtitle")}
          value={t("mobile.import.lang.both")}
          chevron
          onPress={() => {}}
        />
        <ListRow icon="info" title={t("mobile.callLimit.body")} />
      </ListSection>
    );
  },
  note: "Rows grow with the text; the 44 pt minimum holds.",
};

export const SwitchRows: Story = {
  render: function Render() {
    const t = useMobileT();
    const toggle = (titleId: string) => (
      <Switch.Root
        aria-labelledby={titleId}
        defaultChecked
        className="relative h-[1.9375rem] w-[3.1875rem] shrink-0 rounded-full bg-line2 data-[state=checked]:bg-accent"
      >
        <Switch.Thumb className="block size-[1.6875rem] translate-x-0.5 rounded-full bg-white shadow-float transition-transform duration-(--motion-fast) data-[state=checked]:translate-x-[1.3125rem]" />
      </Switch.Root>
    );
    return (
      <ListSection header={t("mobile.tabs.settings")}>
        <ListRow icon="lock" title={t("mobile.record.listening")} trailing={toggle} />
        <ListRow icon="cloud_off" title={t("mobile.record.identifying")} subtitle={t("mobile.record.onPhone")} trailing={toggle} />
      </ListSection>
    );
  },
  note: "The switch is named by the row title (aria-labelledby).",
};
