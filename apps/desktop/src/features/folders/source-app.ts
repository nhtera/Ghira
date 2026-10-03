// SPDX-License-Identifier: Apache-2.0
// The app a file came from (`sourceApp` on a library row): product names, the
// same in every language.
const NAMES: Record<string, string> = { zoom: "Zoom", teams: "Teams", meet: "Meet", plaud: "Plaud", voice_memos: "Voice Memos" };

export const sourceAppName = (id: string | null): string | null => (id ? (NAMES[id] ?? null) : null);
