// SPDX-License-Identifier: Apache-2.0
// Meetings library (D3): built in phase 10/11 on the store; the shell shows
// its frame and the empty state for now.
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { Button, EmptyState, usePlatform } from "@ghi/ui";
import shell from "@ghi/ui/mocks/shell.json";
import { ipc } from "../ipc";
import { Page } from "../shell/page";
import { useAppActions } from "../shell/actions";

export function MeetingsScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const { startRecording } = useAppActions();
  const navigate = useNavigate();
  return (
    <Page
      title={t("nav.meetings")}
      // Counts come from the store in phase 10; sample numbers only on the mock core.
      subtitle={ipc.kind === "mock" ? t("library.subtitle", { context: platform, count: shell.libraryCount.count }) : undefined}
      actions={
        <Button variant="primary" icon="mic" onClick={() => void startRecording()}>
          {t("library.newRecording")}
        </Button>
      }
    >
      <EmptyState kind="library" className="mt-10" onPrimary={() => void startRecording("call")} onSecondary={() => void navigate({ to: "/import" })} />
    </Page>
  );
}
