// SPDX-License-Identifier: Apache-2.0
// Types the `mobile.*` keys for this app only (the desktop never sees them).
import type { MobileOnly } from "@ghi/i18n/mobile";

declare module "@ghi/i18n" {
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  interface ExtraMessages extends MobileOnly {}
}
