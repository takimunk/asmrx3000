import AppKit
import ImageIO
import SwiftUI
import UniformTypeIdentifiers

/// `ASMRX3000 --snapshot out.png [--select tree|bird|river|none] [--orbit 0..1] [--azimuth rad]`
/// runs the engine for a few seconds without an audio device and writes the whole popover
/// (Metal scene and spectrogram included) as a 2x PNG. For visual checks and CI.
enum Snapshot {
    @MainActor
    static func run(path: String, seconds: Double = 11) {
        let args = CommandLine.arguments
        func value(_ flag: String) -> String? {
            args.firstIndex(of: flag).flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
        }
        IconView.still = true
        let model = AppModel()
        let stage = model.stage
        model.audio.orbit = value("--orbit").flatMap(Float.init) ?? 0
        model.audio.birds = value("--birds").flatMap(Float.init) ?? 1
        if let az = value("--azimuth").flatMap(Float.init) { stage.camera.azimuth = az }

        let fps = 30.0
        let selectKind = value("--select") ?? "tree"
        for i in 0..<Int(seconds * fps) {
            model.audio.simulate(seconds: 1 / fps)
            stage.step(dt: 1 / fps)
            if i == 2 {
                // Select something and move it a little, the way a user would.
                switch selectKind {
                case "tree":
                    if let t = stage.things.first(where: { $0.kind == .tree }) {
                        stage.select(t.key)
                        stage.nudge(SIMD3(0.5, 0, 0))
                    }
                case "bird":
                    if let t = stage.things.last(where: { $0.kind == .bird }) {
                        stage.select(t.key)
                        stage.nudge(SIMD3(0, 0, 0.75))
                    }
                case "river":
                    stage.select(.river)
                default:
                    break
                }
            }
        }
        if let h = stage.things.first(where: { $0.kind == .bird && $0.key != stage.selection }) { stage.hover = h.key }

        let scale: Float = 2
        let sceneSize = SIMD2(Float(Popover.width), Float(Popover.sceneHeight))
        let specSize = SIMD2(Float(Popover.width - 32), Float(Popover.spectrogramHeight))
        let sceneRenderer = SceneRenderer(stage: stage)
        let specRenderer = SpectrogramRenderer(stage: stage)
        let tracksRenderer = TracksRenderer(stage: stage)
        let tracksSize = SIMD2(Float(Popover.width - 32 - Popover.iconColumn), Float(Popover.rowHeight) * Float(Track.allCases.count))
        let a = model.audio
        let values = [a.water, a.wind, a.birds, a.orbit, a.volume]
        let hover = SIMD2<Float>(specSize.x * 0.72, specSize.y * 0.38)
        specRenderer.hover = hover
        guard let scene = GPU.shared.image(width: Int(sceneSize.x * scale), height: Int(sceneSize.y * scale), encode: {
                  sceneRenderer.encode($0, target: $1, size: sceneSize, scale: scale)
              }),
              let spec = GPU.shared.image(width: Int(specSize.x * scale), height: Int(specSize.y * scale), encode: {
                  specRenderer.encode($0, target: $1, size: specSize, scale: scale)
              }),
              let tracks = GPU.shared.image(width: Int(tracksSize.x * scale), height: Int(tracksSize.y * scale), encode: {
                  tracksRenderer.encode($0, target: $1, size: tracksSize, scale: scale, values: values)
              })
        else { return }

        let renderer = ImageRenderer(content: Popover(model: model, stills: StillFrames(scene: scene, spectrogram: spec, tracks: tracks, spectrogramHover: hover)))
        renderer.scale = CGFloat(scale)
        guard let image = renderer.cgImage,
              let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL, UTType.png.identifier as CFString, 1, nil)
        else { return }
        CGImageDestinationAddImage(dest, image, nil)
        CGImageDestinationFinalize(dest)
        print("snapshot → \(path)")
        for s in stage.sources { print(s.kind, String(format: "level %.4f freq %.0f", s.level, s.freq)) }
        print("wind", stage.wind.prefix(8).map { String(format: "%.2f", $0) })
    }
}
