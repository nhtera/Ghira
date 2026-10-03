// SPDX-License-Identifier: Apache-2.0
import { Icon } from "../../icons/icon";
import type { Story, StoryMeta } from "../../story";
import { useMobileT } from "../../utils/mobile-t";
import { ListRow, ListSection } from "../list";
import { LargeTitle, NavBar, useLargeTitleCollapse } from "./nav-bar";

export default { title: "Nav bar (iOS)", platform: "ios" } satisfies StoryMeta;

function AddButton() {
  const t = useMobileT();
  return (
    <button type="button" aria-label={t("mobile.import.action")} className="grid min-h-ios-target min-w-ios-target place-items-center text-accent">
      <Icon name="add" size={26} />
    </button>
  );
}

export const LargeTitleExpanded: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <>
        <NavBar title={t("mobile.tabs.meetings")} trailing={<AddButton />} />
        <LargeTitle>{t("mobile.tabs.meetings")}</LargeTitle>
      </>
    );
  },
};

export const Collapsed: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <>
        <NavBar title={t("mobile.tabs.meetings")} collapsed trailing={<AddButton />} />
        <LargeTitle className="sr-only">{t("mobile.tabs.meetings")}</LargeTitle>
      </>
    );
  },
  note: "The large title has scrolled away.",
};

export const WithBack: Story = {
  render: function Render() {
    const t = useMobileT();
    return <NavBar title={t("mobile.detail.notes")} onBack={() => {}} backLabel={t("mobile.tabs.meetings")} large={false} />;
  },
  note: "At large text the back label gives way to the chevron.",
};

export const LongTitle: Story = {
  render: function Render() {
    const t = useMobileT();
    const title = `${t("mobile.callLimit.title")} · ${t("mobile.callLimit.useRoom")}`;
    return (
      <>
        <NavBar title={title} onBack={() => {}} backLabel={t("mobile.tabs.meetings")} />
        <LargeTitle>{title}</LargeTitle>
      </>
    );
  },
  note: "Wraps in the large title; truncates in the bar.",
};

export const CollapsesOnScroll: Story = {
  render: function Render() {
    const t = useMobileT();
    const { collapsed, scrollRef, titleRef } = useLargeTitleCollapse();
    return (
      <div className="flex h-80 flex-col overflow-hidden">
        <NavBar title={t("mobile.tabs.meetings")} collapsed={collapsed} />
        <div ref={scrollRef} tabIndex={0} className="min-h-0 flex-1 overflow-y-auto">
          <LargeTitle ref={titleRef}>{t("mobile.tabs.meetings")}</LargeTitle>
          <ListSection>
            {Array.from({ length: 12 }, (_, i) => (
              <ListRow key={i} title={`${t("mobile.meetings.today")} · ${i + 1}`} chevron onPress={() => {}} />
            ))}
          </ListSection>
        </div>
      </div>
    );
  },
  note: "Scroll the list: the large title folds into the bar.",
};
