// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import models from "../../../mocks/models.json";
import type { Story, StoryMeta } from "../../story";
import { ModelRow, type ModelRowProps, type ModelStatus } from "./model-row";

export default { title: "Model row", width: 780 } satisfies StoryMeta;

const LICENSE: Record<string, string> = {
  "Silero VAD v5": "MIT",
  "Parakeet TDT 0.6B v3": "CC-BY-4.0",
  "Whisper large-v3-turbo": "MIT",
  "Nemotron 3 Diarization": "NVIDIA Open",
  "multilingual-e5-small": "MIT",
  "Qwen3-8B (Q4_K_M)": "Apache-2.0",
  "Qwen3-30B-A3B (Q4)": "Apache-2.0",
};

function Row({ index, status, progress, ...rest }: { index: number; status?: ModelStatus; progress?: number } & Partial<ModelRowProps>) {
  const { i18n } = useTranslation();
  const m = models.models[index]!;
  return (
    <div className="overflow-hidden rounded-row border border-line">
      <ModelRow
        purpose={m.role[i18n.language === "vi" ? 1 : 0]!}
        name={m.m}
        size={m.size}
        memory={m.ram}
        languages={m.lang}
        license={LICENSE[m.m]}
        status={status ?? (m.st as ModelStatus)}
        progress={progress}
        needsMemory="32 GB"
        {...rest}
      />
    </div>
  );
}
const noop = () => {};

export const Installed: Story = { render: () => <Row index={1} onRemove={noop} /> };
export const Downloading: Story = { render: () => <Row index={6} progress={35} onPause={noop} /> };
export const Paused: Story = { render: () => <Row index={5} status="paused" progress={41} onResume={noop} /> };
export const UpdateAvailable: Story = { render: () => <Row index={2} onUpdate={noop} onRemove={noop} /> };
export const Preview: Story = { render: () => <Row index={3} onDownload={noop} /> };
export const IncompatibleHardware: Story = { render: () => <Row index={7} /> };
