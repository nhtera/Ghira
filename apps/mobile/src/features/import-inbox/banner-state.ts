// SPDX-License-Identifier: Apache-2.0
// The "N files are waiting" banner can be hidden. It stays hidden while the
// waiting count is no bigger than it was when the user closed it; a new file
// (a bigger count) brings it back. Not persisted: a restart shows it again.

/** `hidden`: how many were waiting when it was closed (null: not closed). */
export function bannerVisible(waiting: number, hidden: number | null): boolean {
  return waiting > 0 && (hidden === null || waiting > hidden);
}

/**
 * The count to remember: it follows the count down, so a file arriving after
 * some were handled still counts as new. Only while the list is real
 * (`loaded`): locked, or not reloaded yet, the count is 0 because nothing is
 * known, and the same files must not come back as "new" after an unlock.
 */
export function hiddenAfter(
  waiting: number,
  hidden: number | null,
  loaded: boolean,
): number | null {
  return hidden === null || !loaded ? hidden : Math.min(hidden, waiting);
}
