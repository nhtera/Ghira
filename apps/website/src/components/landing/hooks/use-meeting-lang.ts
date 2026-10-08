// SPDX-License-Identifier: Apache-2.0

// The "Meeting language" pick, shared by the live demo and the notes (and
// every switch on the page). The server and the first client render use
// English, so the prerendered HTML hydrates cleanly; the stored pick is
// applied right after (useSyncExternalStore switches to the client snapshot
// once hydrated). Persisted in localStorage; a blocked storage is fine.

import { useSyncExternalStore } from "react";
import { DEFAULT_LANG, LANG_KEY, type Lang } from "@/content/demo-data";

const listeners = new Set<() => void>();
let current: Lang | null = null;

function readStored(): Lang {
  try {
    return localStorage.getItem(LANG_KEY) === "vi" ? "vi" : DEFAULT_LANG;
  } catch {
    return DEFAULT_LANG;
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const getSnapshot = (): Lang => (current ??= readStored());
const getServerSnapshot = (): Lang => DEFAULT_LANG;

export function setMeetingLang(lang: Lang) {
  current = lang;
  try {
    localStorage.setItem(LANG_KEY, lang);
  } catch {
    // Storage is blocked: the pick lasts for this visit only.
  }
  for (const listener of listeners) listener();
}

export function useMeetingLang(): [Lang, (lang: Lang) => void] {
  return [useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot), setMeetingLang];
}
