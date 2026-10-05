// SPDX-License-Identifier: Apache-2.0
// LAN sync, Swift side (phase 15, doc 07): the camera scan of the desktop's
// pairing code and the Bonjour browse for its service. Contract stubs for
// now: the real AVCaptureSession scanner and NWBrowser arrive with slice
// 15-I, calling `ghi_ios_qr_scanned` / `ghi_ios_browse_found` (C ABI in
// include/ghi_ios.h). Nothing is started, so no permission prompt appears.

import Foundation

@_cdecl("ghi_swift_qr_scan_start")
public func ghiSwiftQrScanStart() {}

@_cdecl("ghi_swift_qr_scan_stop")
public func ghiSwiftQrScanStop() {}

@_cdecl("ghi_swift_browse_start")
public func ghiSwiftBrowseStart() {}

@_cdecl("ghi_swift_browse_stop")
public func ghiSwiftBrowseStop() {}
