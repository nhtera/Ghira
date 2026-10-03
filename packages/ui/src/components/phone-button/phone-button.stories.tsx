// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { useMobileT } from "../../utils/mobile-t";
import { PhoneButton } from "./phone-button";

export default { title: "Phone button (iOS)", platform: "ios" } satisfies StoryMeta;

function Variants({ disabled }: { disabled?: boolean }) {
  const t = useMobileT();
  return (
    <div className="flex flex-col gap-3 p-4">
      <PhoneButton disabled={disabled}>{t("mobile.tabs.record")}</PhoneButton>
      <PhoneButton variant="secondary" icon="search" disabled={disabled}>
        {t("mobile.tabs.search")}
      </PhoneButton>
      <PhoneButton variant="ghost" disabled={disabled}>
        {t("mobile.tabs.settings")}
      </PhoneButton>
      <PhoneButton inline variant="secondary" disabled={disabled}>
        {t("mobile.tabs.meetings")}
      </PhoneButton>
    </div>
  );
}

export const Variant: Story = { render: () => <Variants /> };
export const Disabled: Story = { render: () => <Variants disabled /> };
