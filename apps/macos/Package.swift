// swift-tools-version:5.9
import Foundation
import PackageDescription

// The Rust core is built first (see scripts/build-macos.sh) into core/target/release.
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let rustLibDir = root.appendingPathComponent("../../core/target/release").standardizedFileURL.path
// Link the core's static library by path: with `-lasmr_core` the linker prefers the cdylib
// that cargo also leaves in that directory, and the app then loads it from this machine's
// build tree at launch (and crashes on any other Mac).
let rustStaticLib = rustLibDir + "/libasmr_core.a"

let package = Package(
    name: "ASMRX3000",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "CAsmr", path: "Sources/CAsmr"),
        .executableTarget(
            name: "ASMRX3000",
            dependencies: ["CAsmr"],
            path: "Sources/ASMRX3000",
            linkerSettings: [
                .unsafeFlags([rustStaticLib]),
                .linkedFramework("CoreMotion"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
            ]
        ),
    ]
)
