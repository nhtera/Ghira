// swift-tools-version:5.9
// SPDX-License-Identifier: Apache-2.0

import PackageDescription

let package = Package(
    name: "GhiAudioMac",
    platforms: [.macOS(.v14)],
    products: [
        // Linked into ghi-audio by crates/ghi-audio/build.rs; the C ABI is include/ghi_audio_mac.h.
        .library(name: "GhiAudioMac", type: .static, targets: ["GhiAudioMac"]),
        .executable(name: "ghi-audio-mac-probe", targets: ["ghi-audio-mac-probe"]),
    ],
    targets: [
        // swift-tools-version 5.9 compiles in Swift 5 language mode.
        .target(name: "GhiAudioMac"),
        // Live check tool. The embedded Info.plist carries the TCC usage strings
        // (microphone, System Audio Recording) that a bare command-line binary lacks.
        .executableTarget(
            name: "ghi-audio-mac-probe",
            dependencies: ["GhiAudioMac"],
            exclude: ["Info.plist"],
            linkerSettings: [
                .unsafeFlags([
                    "-Xlinker", "-sectcreate", "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist", "-Xlinker", "Sources/ghi-audio-mac-probe/Info.plist",
                ])
            ]
        ),
    ]
)
