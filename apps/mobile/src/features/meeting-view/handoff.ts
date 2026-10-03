// SPDX-License-Identifier: Apache-2.0
// Where this slice hands off to others: the cloud send sheet (16-J, mounted
// globally) and the native share sheet (16-G). One file, one edit if a contract moves.
import { openCloudSheet as open } from "../cloud-sheet";
import { ipc } from "../../ipc";

/** The cloud sheet over the current screen; `onSent` runs after a send (reload the notes). */
export function openCloudSheet(
  meetingId: string,
  cloudLocked: boolean | undefined,
  onSent: () => void,
) {
  open({ meetingId, cloudLocked, onSent });
}

/** Export through the native share sheet. */
export async function shareExport(
  meeting: string,
  markdown: boolean,
): Promise<boolean> {
  const r = await ipc.commands
    .shareMeetingExport(meeting, markdown ? "md" : "txt")
    .catch(() => null);
  return r?.status === "ok";
}

/**
 * The export text for the "Copy" fallback, in the language Rust uses for the
 * share (the meeting language setting). Fetched ahead, so the Copy tap can call
 * `writeText` itself, inside the gesture.
 */
export async function exportText(
  meeting: string,
  markdown: boolean,
): Promise<string | null> {
  const settings = await ipc.commands.getSettings().catch(() => null);
  const vietnamese =
    settings?.status === "ok" && settings.data.meetingLanguage === "vi";
  const r = await ipc.commands
    .meetingAsText(meeting, markdown, {
      notes: true,
      transcript: true,
      vietnamese,
    })
    .catch(() => null);
  return r?.status === "ok" ? r.data : null;
}
