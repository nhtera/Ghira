// SPDX-License-Identifier: Apache-2.0
// Export as Markdown or plain text through the native share sheet. If the
// core cannot present it, the sheet says so and offers an explicit Copy.
import { Button, ListRow, ListSection, Sheet } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { exportText, shareExport } from "./handoff";

export type ShareSheetProps = {
  open: boolean;
  onClose: () => void;
  meeting: string;
};

type Fallback = { text: string | null } | { copied: true } | null;

export function ShareSheet({ open, onClose, meeting }: ShareSheetProps) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState<Fallback>(null);

  const share = async (markdown: boolean) => {
    setFailed(null);
    if (await shareExport(meeting, markdown)) return onClose();
    // Fetch the text now: Copy then needs nothing async in its tap.
    setFailed({ text: await exportText(meeting, markdown) });
  };
  const close = () => {
    setFailed(null);
    onClose();
  };

  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && close()}
      title={t("mobile.detail.shareTitle")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
    >
      <ListSection className="mx-0">
        <ListRow
          icon="description"
          title={t("mobile.detail.shareMarkdown")}
          onPress={() => void share(true)}
        />
        <ListRow
          icon="description"
          title={t("mobile.detail.shareText")}
          onPress={() => void share(false)}
        />
      </ListSection>
      {failed && (
        <div className="mt-2 flex flex-col items-start gap-2">
          {"copied" in failed ? (
            <p role="status" className="text-ios-footnote m-0 text-muted">
              {t("mobile.detail.copied")}
            </p>
          ) : (
            <>
              <p role="alert" className="text-ios-footnote m-0">
                {failed.text === null
                  ? t("mobile.detail.shareFailed")
                  : t("mobile.detail.shareFailedCopy")}
              </p>
              {failed.text !== null && (
                <Button
                  icon="content_copy"
                  className="min-h-ios-target px-4"
                  onClick={() => {
                    const text = failed.text ?? "";
                    // Straight from the tap, so the webview allows it.
                    navigator.clipboard.writeText(text).then(
                      () => setFailed({ copied: true }),
                      () => setFailed({ text: null }),
                    );
                  }}
                >
                  {t("mobile.detail.shareCopy")}
                </Button>
              )}
            </>
          )}
        </div>
      )}
    </Sheet>
  );
}
