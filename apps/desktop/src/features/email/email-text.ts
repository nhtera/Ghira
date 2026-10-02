// SPDX-License-Identifier: Apache-2.0
// What Copy puts on the clipboard: the subject, a blank line, the body.
export const emailText = (subject: string, body: string) => `${subject.trim()}\n\n${body.trim()}`;
