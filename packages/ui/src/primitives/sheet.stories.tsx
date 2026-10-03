// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import { Button } from "./button";
import { Sheet, type SheetDetent } from "./sheet";
import type { Story, StoryMeta } from "../story";
import { ListRow, ListSection } from "../components/list";
import { useMobileT } from "../utils/mobile-t";

export default { title: "Sheet (iOS)", platform: "ios" } satisfies StoryMeta;

function Demo({ detent, dismissible = true, long = false }: { detent: SheetDetent; dismissible?: boolean; long?: boolean }) {
  const t = useMobileT();
  const [open, setOpen] = useState(true);
  return (
    <>
      <Button variant="primary" onClick={() => setOpen(true)}>
        {t("mobile.record.resumeRecording")}
      </Button>
      <Sheet
        open={open}
        onOpenChange={setOpen}
        detent={detent}
        dismissible={dismissible}
        title={t("mobile.record.pausedCall.title")}
        description={t("mobile.record.pausedCall.body", { time: "12:41" })}
        closeLabel={t("mobile.sheet.close")}
        handleLabel={t("mobile.sheet.handle")}
        footer={
          <>
            <button type="button" onClick={() => setOpen(false)} className="text-ios-headline min-h-12 w-full rounded-xl bg-accent px-4 py-2 text-on-accent">
              {t("mobile.record.resumeRecording")}
            </button>
            <button type="button" onClick={() => setOpen(false)} className="text-ios-callout min-h-ios-target w-full rounded-xl px-4 py-2 font-semibold text-rec-ink">
              {t("mobile.record.stopAndSave")}
            </button>
          </>
        }
      >
        {long && (
          <ListSection className="mx-0">
            {Array.from({ length: 14 }, (_, i) => (
              <ListRow key={i} icon="record_voice_over" title={`${t("mobile.record.speaker")} ${i + 1}`} subtitle={t("mobile.record.listening")} />
            ))}
          </ListSection>
        )}
      </Sheet>
    </>
  );
}

export const Medium: Story = { overlay: true, render: () => <Demo detent="medium" /> };
export const Large: Story = { overlay: true, render: () => <Demo detent="large" long />, note: "Content scrolls; the footer stays pinned." };
export const NotDismissible: Story = { overlay: true, render: () => <Demo detent="medium" dismissible={false} />, note: "Consent: no close button, Escape and the scrim do nothing." };
