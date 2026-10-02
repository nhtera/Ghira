// SPDX-License-Identifier: Apache-2.0
// What the export sheet asks the core for. Subtitles are transcript-only;
// headings follow the app language.
import type { ExportContent, ExportFormat } from "../../bindings";

export const FORMATS: ExportFormat[] = ["markdown", "docx", "text", "srt", "vtt"];
export const isSubtitles = (f: ExportFormat) => f === "srt" || f === "vtt";

export function exportContent(format: ExportFormat, notes: boolean, transcript: boolean, language: string): ExportContent {
  const vietnamese = language.startsWith("vi");
  return isSubtitles(format) ? { notes: false, transcript: true, vietnamese } : { notes, transcript, vietnamese };
}

/** Something to export: subtitles always have the transcript; the rest need a part ticked. */
export const canExport = (format: ExportFormat, notes: boolean, transcript: boolean) => isSubtitles(format) || notes || transcript;
