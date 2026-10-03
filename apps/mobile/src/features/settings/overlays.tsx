// SPDX-License-Identifier: Apache-2.0
// What shows above every screen: the cloud send sheet host, the share-inbox
// banner and sheet, and (last, so it covers everything) the app-lock gate.
// Mounted once in shell/root-view.tsx.
import { AppLockGate } from "../app-lock";
import { CloudSheetHost } from "../cloud-sheet";
import { ImportInbox } from "../import-inbox";

export function GlobalOverlays() {
  return (
    <>
      <CloudSheetHost />
      <ImportInbox />
      <AppLockGate />
    </>
  );
}
