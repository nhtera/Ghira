// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { citedMs, pick, sampleNotes } from "../avatar/sample-data";
import type { Story, StoryMeta } from "../../story";
import { ActionItem, type ActionItemProps } from "./action-item";

export default { title: "Action item", width: 560 } satisfies StoryMeta;

type Sample = { o: number; en: string; vi: string; due: [string, string]; c: number[] };
const items = sampleNotes.actionItems as Sample[];
const OWNERS = [
  { name: "Me", colorSlot: 1, isMe: true },
  { name: "Linh", colorSlot: 2 },
  { name: "Minh", colorSlot: 4 },
  { name: "Sarah", colorSlot: 8 },
];

function Item({ i, ...rest }: { i: number } & Partial<ActionItemProps>) {
  const { i18n } = useTranslation();
  const lang = i18n.language;
  const a = items[i]!;
  const [done, setDone] = useState(rest.done ?? false);
  return (
    <ActionItem
      text={pick(a, lang)}
      owner={a.o >= 0 ? OWNERS[a.o] : null}
      due={a.due[0] ? { text: lang === "vi" ? a.due[1] : a.due[0] } : undefined}
      citations={a.c.map((c) => ({ timeMs: citedMs(c) }))}
      {...rest}
      done={done}
      onToggle={setDone}
    />
  );
}

export const Open: Story = { render: () => <Item i={0} /> };
export const Done: Story = { render: () => <Item i={1} done /> };
export const OwnerUnassigned: Story = { render: () => <Item i={3} /> };
export const DueSoon: Story = {
  render: () => {
    const Soon = () => {
      const { i18n } = useTranslation();
      return <Item i={2} due={{ text: i18n.language === "vi" ? "Hạn ngày mai" : "Due tomorrow", tone: "soon" }} />;
    };
    return <Soon />;
  },
};
export const Overdue: Story = {
  render: () => {
    const Late = () => {
      const { i18n } = useTranslation();
      return <Item i={1} owner={OWNERS[0]} due={{ text: i18n.language === "vi" ? "Quá hạn · 2 ngày" : "Overdue · 2 days", tone: "overdue" }} />;
    };
    return <Late />;
  },
};
