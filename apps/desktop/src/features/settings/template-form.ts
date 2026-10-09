// SPDX-License-Identifier: Apache-2.0
// The template editor's rules, as the core checks them on save (Template::from_editor):
// the form disables Save with the same words, so a refusal is rare.
import type { TemplateForm } from "../../bindings";

export const MAX_NAME = 60;
export const MAX_GUIDANCE = 400;
export const MAX_SECTION_TITLE = 60;
export const MAX_INSTRUCTION = 200;
export const MAX_SECTIONS = 8;
export const MAX_TEMPLATES = 20;

const len = (s: string) => Array.from(s.split(/\s+/).filter(Boolean).join(" ")).length;

export type FormIssue = "nameRequired" | "guidanceTooLong" | "titleRequired" | null;

/** The first thing wrong with the form (the core refuses the same). */
export function formIssue(f: TemplateForm): FormIssue {
  if (len(f.name) < 1 || len(f.name) > MAX_NAME) return "nameRequired";
  if (len(f.guidance) > MAX_GUIDANCE) return "guidanceTooLong";
  if (f.sections.length > MAX_SECTIONS) return "titleRequired";
  for (const s of f.sections) {
    if (len(s.title) < 1 || len(s.title) > MAX_SECTION_TITLE || len(s.instruction) < 1 || len(s.instruction) > MAX_INSTRUCTION) return "titleRequired";
  }
  return null;
}

export const emptyForm = (language: string): TemplateForm => ({
  name: "",
  language,
  guidance: "",
  sections: [{ id: null, title: "", instruction: "" }],
});
