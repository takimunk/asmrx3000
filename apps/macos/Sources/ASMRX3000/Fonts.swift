import CoreText
import Foundation

/// The app's own face, Coppice (tools/font/coppice.py). Inside the bundle ATSApplicationFontsPath
/// loads it; runs from .build (snapshots, bench) register it from the source tree.
enum Fonts {
    static func register() {
        if Bundle.main.url(forResource: "Fonts", withExtension: nil) != nil { return }
        let dir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../Resources/Fonts").standardizedFileURL
        let files = (try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)) ?? []
        for url in files where url.pathExtension == "ttf" {
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }
}
