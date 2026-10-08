// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import { ICON_NAMES } from "../icons/icon-data";
import { Icon } from "../icons/icon";
import { PHONE_ICONS } from "../icons/phone-icons";
import type { Story, StoryMeta } from "../story";
import { Button } from "./button";
import { Dialog } from "./dialog";
import { Menu } from "./menu";
import { Popover } from "./popover";
import { Segmented } from "./segmented";
import { Select } from "./select";
import { ToastPreview, ToastView } from "./toast";
import { Tooltip } from "./tooltip";

export default { title: "Primitives", width: 560 } satisfies StoryMeta;

export const Buttons: Story = {
  render: () => (
    <div className="flex flex-col gap-3">
      {(["sm", "md", "lg"] as const).map((size) => (
        <div key={size} className="flex flex-wrap items-center gap-2">
          <Button size={size} variant="primary" icon="videocam">
            Record call
          </Button>
          <Button size={size}>Export</Button>
          <Button size={size} variant="ghost">
            Skip
          </Button>
          <Button size={size} variant="danger" icon="delete">
            Delete
          </Button>
          <Button size={size} icon="more_horiz" aria-label="More" />
          <Button size={size} disabled>
            Save
          </Button>
        </div>
      ))}
    </div>
  ),
};

function SegmentedDemo() {
  const [v, setV] = useState<"light" | "dark" | "system">("light");
  return (
    <Segmented
      label="Theme"
      value={v}
      onChange={setV}
      options={[
        { value: "light", label: "Light", icon: "light_mode" },
        { value: "dark", label: "Dark", icon: "dark_mode" },
        { value: "system", label: "System" },
      ]}
    />
  );
}
export const SegmentedControl: Story = { render: () => <SegmentedDemo /> };

function SelectDemo() {
  const [v, setV] = useState("claude-sonnet-5-5");
  const options = [
    { value: "claude-sonnet-5-5", label: "claude-sonnet-5-5 · $2 / $10" },
    { value: "claude-haiku-5-5", label: "claude-haiku-5-5 · $0.10 / $0.50" },
    { value: "claude-opus-5-5", label: "claude-opus-5-5 · $4 / $20" },
  ];
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Select label="Model" value={v} onChange={setV} options={options} />
      <Select label="Model (small)" size="sm" value={v} onChange={setV} options={options} />
      <Select label="Model (disabled)" disabled value={v} onChange={setV} options={options} />
    </div>
  );
}
export const SelectControl: Story = { render: () => <SelectDemo /> };

export const Icons: Story = {
  note: "Material Symbols Rounded on mac; Fluent 20 Regular on Windows (?platform=win).",
  render: () => (
    <div className="grid grid-cols-8 gap-3 text-muted">
      {ICON_NAMES.filter((n) => !(PHONE_ICONS as readonly string[]).includes(n)).map((n) => (
        <span key={n} title={n} className="grid place-items-center">
          <Icon name={n} size={20} label={n} />
        </span>
      ))}
    </div>
  ),
};

export const Toasts: Story = {
  render: () => (
    <ToastPreview label="Notifications">
      <ToastView title="Consent message copied. Paste it in the meeting chat." />
      <ToastView tone="success" title="Notes are ready" />
      <ToastView tone="warning" title="Disk almost full · about 2 h of recording left" />
      <ToastView title="Meeting deleted" action={{ label: "Undo", altText: "Undo delete", onAction: () => {} }} />
    </ToastPreview>
  ),
};

export const TooltipOnFocus: Story = {
  render: () => (
    <Tooltip content="Mark moment (⌘M)">
      <Button icon="star" aria-label="Mark moment" />
    </Tooltip>
  ),
};

export const PopoverOpen: Story = {
  overlay: true,
  render: () => (
    <Popover open label="Rename speaker" trigger={<Button>Speaker 2</Button>}>
      <p className="text-body m-0">Who is this?</p>
    </Popover>
  ),
};

export const MenuOpen: Story = {
  overlay: true,
  render: () => (
    <Menu
      label="More"
      trigger={<Button icon="more_horiz" aria-label="More" />}
      items={[
        { label: "Regenerate notes", icon: "refresh", onSelect: () => {} },
        { label: "Copy Markdown", icon: "content_copy", onSelect: () => {}, hint: "⌘⇧C" },
        { kind: "separator" },
        { label: "Delete meeting", icon: "delete", danger: true, onSelect: () => {} },
      ]}
    />
  ),
};

export const DialogOpen: Story = {
  overlay: true,
  note: "A sheet under the title bar on mac; centered on Windows.",
  render: () => (
    <Dialog
      open
      onOpenChange={() => {}}
      title="Export notes"
      description="Choose a format. Files are written to the folder you pick."
      footer={
        <>
          <Button>Cancel</Button>
          <Button variant="primary">Export</Button>
        </>
      }
    />
  ),
};

export const DialogCentered: Story = {
  overlay: true,
  note: "placement=\"center\": centered on mac too (the crash-recovery dialog).",
  render: () => (
    <Dialog open onOpenChange={() => {}} placement="center" title="Closed unexpectedly" description="Your recording was saved." footer={<Button variant="primary">Recover</Button>} />
  ),
};
