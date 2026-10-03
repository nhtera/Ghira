// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { useMobileT } from "../../utils/mobile-t";
import { Banner } from "./banner";

export default { title: "Banner (iOS)", platform: "ios" } satisfies StoryMeta;

export const Pocket: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <Banner variant="warning" icon="hearing_disabled" title={t("mobile.record.pocket.title")}>
        {t("mobile.record.pocket.hint")}
      </Banner>
    );
  },
};

export const TooHot: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <Banner variant="warning" icon="thermostat" title={t("mobile.record.hot.title")}>
        {t("mobile.record.hot.body")}
      </Banner>
    );
  },
};

export const RecordOnly: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <Banner variant="info" icon="mic" title={t("mobile.record.recordOnly.title")}>
        {t("mobile.record.recordOnly.body")}
      </Banner>
    );
  },
};

export const WithActionAndDismiss: Story = {
  render: function Render() {
    const t = useMobileT();
    return (
      <Banner
        variant="warning"
        icon="phone_paused"
        title={t("mobile.record.pausedCall.title")}
        action={{ label: t("mobile.record.resume"), onPress: () => {} }}
        onDismiss={() => {}}
        dismissLabel={t("mobile.banner.dismiss")}
      >
        {t("mobile.record.pausedCall.body", { time: "12:41" })}
      </Banner>
    );
  },
};
