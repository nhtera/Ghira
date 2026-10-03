// SPDX-License-Identifier: Apache-2.0
export const APP: string;
export const PREFIX: string;
export const PLURAL: RegExp;
export function varsOf(s: string): string[];
export function format(s: string, order: string[]): string;
export function build(langs: { en: Record<string, string>; vi: Record<string, string> }): string;
