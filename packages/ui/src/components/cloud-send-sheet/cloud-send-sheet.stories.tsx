// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import sample from "../../../mocks/sample-meeting.json";
import type { Story, StoryMeta } from "../../story";
import { CloudSendSheet, type CloudSendSheetProps, type CloudSendState } from "./cloud-send-sheet";

export default { title: "Cloud send sheet", width: 640 } satisfies StoryMeta;

const PROVIDERS = [
  { id: "anthropic", name: "Claude" },
  { id: "openai", name: "OpenAI" },
];

function Demo({ state, redact: initialRedact = true }: { state: CloudSendState; redact?: boolean }) {
  const [redact, setRedact] = useState(initialRedact);
  const [providerId, setProviderId] = useState("anthropic");
  const [open, setOpen] = useState(true);
  const props: CloudSendSheetProps = {
    open,
    onOpenChange: setOpen,
    state,
    providers: PROVIDERS,
    providerId,
    onProviderChange: setProviderId,
    words: 9840,
    tokens: 13100,
    cost: "$0.04",
    redact,
    onRedactChange: setRedact,
    rawText: sample.redaction.raw,
    redactedText: sample.redaction.redacted,
    onSend: () => {},
    onKeepLocal: () => setOpen(false),
  };
  return <CloudSendSheet {...props} />;
}

export const Default: Story = { overlay: true, render: () => <Demo state="default" /> };
export const RedactionOff: Story = { overlay: true, render: () => <Demo state="default" redact={false} /> };
export const Sending: Story = { overlay: true, render: () => <Demo state="sending" /> };
export const Sent: Story = { overlay: true, render: () => <Demo state="sent" /> };
export const Failed: Story = { overlay: true, render: () => <Demo state="failed" />, note: "Falls back to the local notes." };
