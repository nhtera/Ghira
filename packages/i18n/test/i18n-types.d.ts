// SPDX-License-Identifier: Apache-2.0
// What the iOS app declares in apps/mobile/src/i18n-types.d.ts, for these tests.
import type { MobileOnly } from "../src/mobile-entry";

declare module "../src/index" {
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  interface ExtraMessages extends MobileOnly {}
}
