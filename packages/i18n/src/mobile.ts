// SPDX-License-Identifier: Apache-2.0
// The iOS app's strings: every locales/mobile/*.{en,vi}.json merged under the
// `mobile.*` namespace. Kept out of the desktop bundle (`initI18n`); the
// mobile app starts with `initMobileI18n`.
//
// - `_base.*`  extracted from the mobile design (scripts/extract-mobile.mjs)
// - `record.*`, `meetings.*`, `settings.*`  hand-written by the 16-H/I/J UI slices
// - `shell.*`, `chips.*`, `ios.*`, `sync.*`  hand-written; `mobile.ios.*` also feeds
//   native/ios/Shared/Localizable.xcstrings (scripts/gen-ios-strings.mjs)
// A new file in locales/mobile must be added to MOBILE_FILES (a test checks).
import baseEn from "../locales/mobile/_base.en.json";
import baseVi from "../locales/mobile/_base.vi.json";
import chipsEn from "../locales/mobile/chips.en.json";
import chipsVi from "../locales/mobile/chips.vi.json";
import iosEn from "../locales/mobile/ios.en.json";
import iosVi from "../locales/mobile/ios.vi.json";
import meetingsEn from "../locales/mobile/meetings.en.json";
import meetingsVi from "../locales/mobile/meetings.vi.json";
import recordEn from "../locales/mobile/record.en.json";
import recordVi from "../locales/mobile/record.vi.json";
import settingsEn from "../locales/mobile/settings.en.json";
import settingsVi from "../locales/mobile/settings.vi.json";
import shellEn from "../locales/mobile/shell.en.json";
import shellVi from "../locales/mobile/shell.vi.json";
import syncEn from "../locales/mobile/sync.en.json";
import syncVi from "../locales/mobile/sync.vi.json";

/** The file stems in locales/mobile (without `.en.json` / `.vi.json`). */
export const MOBILE_FILES = ["_base", "chips", "ios", "meetings", "record", "settings", "shell", "sync"] as const;

export type MobileOnly = typeof baseEn &
  typeof chipsEn &
  typeof iosEn &
  typeof meetingsEn &
  typeof recordEn &
  typeof settingsEn &
  typeof shellEn &
  typeof syncEn;

type Tree = { [key: string]: string | Tree };

/** Deep merge; two files defining the same key is a bug. */
export function mergeTrees(trees: Tree[], path = ""): Tree {
  const out: Tree = {};
  for (const tree of trees) {
    for (const [k, v] of Object.entries(tree)) {
      const cur = out[k];
      if (cur === undefined) out[k] = v;
      else if (typeof cur === "object" && typeof v === "object") {
        out[k] = mergeTrees([cur, v], `${path}${k}.`);
      } else throw new Error(`mobile locales define ${path}${k} twice`);
    }
  }
  return out;
}

export const mobileLocales = {
  en: mergeTrees([baseEn, chipsEn, iosEn, meetingsEn, recordEn, settingsEn, shellEn, syncEn]) as unknown as MobileOnly,
  vi: mergeTrees([baseVi, chipsVi, iosVi, meetingsVi, recordVi, settingsVi, shellVi, syncVi]) as unknown as MobileOnly,
};
