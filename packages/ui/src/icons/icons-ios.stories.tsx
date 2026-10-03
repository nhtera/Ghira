// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../story";
import { Icon } from "./icon";
import { PHONE_ICONS } from "./phone-icons";

export default { title: "Icons (iOS)", platform: "ios" } satisfies StoryMeta;

export const PhoneSet: Story = {
  render: () => (
    <ul className="m-0 grid list-none grid-cols-6 gap-3 p-0 text-muted">
      {PHONE_ICONS.map((n) => (
        <li key={n} title={n} className="grid place-items-center">
          <Icon name={n} size={24} label={n} />
        </li>
      ))}
    </ul>
  ),
};
