// SPDX-License-Identifier: Apache-2.0
// Meetings library (D3 basics) with processing (D5): rows grouped by day, the
// in-place stepper for meetings being processed, and "Name your speakers".
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, EmptyState, useToast, usePlatform } from "@ghi/ui";
import shell from "@ghi/ui/mocks/shell.json";
import { ipc } from "../ipc";
import { LibraryList } from "../features/library/library-list";
import { MEETINGS_KEY, useMeetings } from "../features/library/use-meetings";
import { NameSpeakers } from "../features/processing/name-speakers";
import { ProcessingPanel } from "../features/processing/processing-panel";
import { useProcessing } from "../features/processing/processing-store";
import { adapter } from "../features/processing/speakers-adapter";
import { Page } from "../shell/page";
import { useAppActions } from "../shell/actions";

/** Voices still unnamed in the meeting that just finished (empty until the adapter has any). */
function useUnnamed(meeting: string | undefined) {
  const q = useQuery({ queryKey: ["unnamed-speakers", meeting], enabled: !!meeting, queryFn: () => adapter.unnamed(meeting!) });
  const [named, setNamed] = useState<string[]>([]);
  const left = useMemo(() => (q.data ?? []).filter((s) => !named.includes(s.gid)), [q.data, named]);
  return { left, loaded: q.isSuccess, markNamed: (gid: string) => setNamed((n) => [...n, gid]) };
}

export function MeetingsScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const { startRecording } = useAppActions();
  const navigate = useNavigate();
  const meetings = useMeetings();
  const client = useQueryClient();
  const forget = useProcessing((s) => s.forget);
  const { show } = useToast();
  const remove = useCallback(
    async (id: string) => {
      const r = await ipc.commands.deleteMeeting(id);
      if (r.status === "error") return show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      forget(id);
      show({ title: t("library.deleted", { count: 1 }) });
      void client.invalidateQueries({ queryKey: MEETINGS_KEY });
    },
    [client, show, t, forget],
  );
  const processing = useProcessing((s) => s.meetings);
  const finished = useProcessing((s) => s.finished);
  const clearFinished = useProcessing((s) => s.clearFinished);
  const rows = meetings.data ?? [];

  // The newest meeting whose notes just finished gets the naming cards.
  const naming = finished[finished.length - 1];
  const { left, loaded, markNamed } = useUnnamed(naming);
  useEffect(() => {
    if (naming && loaded && left.length === 0) clearFinished(naming);
  }, [naming, loaded, left.length, clearFinished]);

  const progress = Object.fromEntries(Object.entries(processing).map(([id, p]) => [id, p.progress ?? undefined]));
  const needsNames = useMemo(() => new Set(naming && left.length > 0 ? [naming] : []), [naming, left.length]);
  const open = useCallback((id: string) => void navigate({ to: "/meetings/$id/$tab", params: { id, tab: "notes" } }), [navigate]);
  const titleOf = (id: string) => rows.find((r) => r.gid === id)?.title;
  const retry = async (id: string) => {
    const r = await ipc.commands.retryMeeting(id);
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    else if (r.data === 0) show({ title: t("library.nothingToRetry") });
    void meetings.refetch();
  };
  const processingIds = Object.keys(processing);

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
      {processingIds.map((id) => (
        <ProcessingPanel key={id} title={titleOf(id)} processing={processing[id]!} waitingForModels={rows.find((r) => r.gid === id)?.job?.waitingForModels} />
      ))}
      {naming && left.length > 0 && <NameSpeakers meeting={naming} speakers={left} onDone={markNamed} onSkipAll={() => clearFinished(naming)} />}
      {meetings.isSuccess && rows.length === 0 && processingIds.length === 0 ? (
        <EmptyState kind="library" className="mt-10" onPrimary={() => void startRecording("call")} onSecondary={() => void navigate({ to: "/import" })} />
      ) : (
        <LibraryList rows={rows} progress={progress} needsNames={needsNames} onOpen={open} onDelete={(id) => void remove(id)} onRetry={(id) => void retry(id)} />
      )}
    </Page>
  );
}
