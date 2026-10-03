// SPDX-License-Identifier: Apache-2.0
// Recipients and "Open in Mail" for the follow-up email.
import { useQuery } from "@tanstack/react-query";
import type { MeetingContact } from "../../bindings";
import { ipc } from "../../ipc";

/** Hands the draft to the mail app: an error message, or whether the body had to be shortened to fit. */
export const openMailDraft = async (to: string[], subject: string, body: string): Promise<{ error: string } | { truncated: boolean }> => {
  const r = await ipc.commands.openMailDraft(to, subject, body);
  return r.status === "error" ? { error: r.error } : { truncated: r.data.truncated };
};

/** The people the follow-up can go to: the calendar event's attendees, with addresses when the invite had them. */
export function useMeetingContacts(meeting: string, enabled: boolean) {
  return useQuery({
    queryKey: ["meeting-contacts", meeting],
    enabled,
    queryFn: async (): Promise<MeetingContact[]> => {
      const r = await ipc.commands.meetingContacts(meeting);
      return r.status === "ok" ? r.data : [];
    },
  });
}
