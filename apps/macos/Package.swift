// swift-tools-version:5.9
import Foundation
import PackageDescription

// The Rust core is built first (see scripts/build-macos.sh) into core/target/release.
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let rustLibDir = root.appendingPathComponent("../../core/target/release").standardizedFileURL.path

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
                .unsafeFlags(["-L", rustLibDir]),
                .linkedFramework("CoreMotion"),
                .linkedFramework("AVFoundation"),
            ]
        ),
    ]
)
