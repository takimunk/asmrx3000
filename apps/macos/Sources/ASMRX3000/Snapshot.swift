import AppKit
import CAsmr
import ImageIO
import simd
import SwiftUI
import UniformTypeIdentifiers

/// `ASMRX3000 --snapshot out.png [--select tree|bird|river|none] [--expand water|wind|none] [--more] [--spectrogram] [--orbit 0..1]
///  [--azimuth rad] [--set key=value ...] [--scene file.json] [--gaze yaw°,pitch°]
///  [--flight speed] [--seed n] [--steer yaw°]`
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
        // --gaze yaw,pitch in degrees (yaw + = left, pitch + = up), as if AirPods were on.
        if let g = value("--gaze")?.split(separator: ",").compactMap({ Float($0) }), g.count == 2 {
            stage.gazeOverride = (g[0] * .pi / 180, g[1] * .pi / 180)
        }

        // --set key=value (repeatable) sets any control, e.g. --set rain.rate=8 --set env.time_of_day=22.
        var tree: [Control] = []
        model.audio.readControls(&tree)
        for (i, a) in args.enumerated() where a == "--set" && i + 1 < args.count {
            let kv = args[i + 1].split(separator: "=")
            guard kv.count == 2, let v = Float(kv[1]), let c = tree.first(where: { $0.key == kv[0] }) else {
                print("unknown control: \(args[i + 1])")
                continue
            }
            model.audio.setControl(c.id, v)
        }

        // --flight <speed m/s>: fly through the streamed landscape.
        if let seed = value("--seed").flatMap(Float.init) {
            model.audio.setControl(UInt32(ASMR_CONTROL_FLIGHT_SEED), seed)
        }
        if let speed = value("--flight").flatMap(Float.init) {
            model.audio.setControl(UInt32(ASMR_CONTROL_FLIGHT_SPEED), speed)
            model.audio.flying = true
        }

        // --steer yaw° (+ = left): an AirPods head pose held from the 3rd second on, so flight
        // mode steers by head (flight.steer_by_head).
        let steer = value("--steer").flatMap(Float.init).map { $0 * .pi / 180 }
        let fps = 30.0
        let selectKind = value("--select") ?? "tree"
        for i in 0..<Int(seconds * fps) {
            if let yaw = steer {
                let y = Double(i) / fps < 3 ? 0 : yaw
                model.audio.setHeadSample(HeadTracker.Sample(q: simd_quatf(angle: y, axis: SIMD3(0, 0, 1)), w: .zero, age: 0.01))
            }
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
        // --prompt "…": the offline mock answers, as a model would, before the frame is taken.
        if let words = value("--prompt"),
           let request = try? PromptRequest(text: words, promptJSON: model.audio.promptJSON(), state: model.audio.stateJSON()),
           let reply = try? MockProvider().reply(request) {
            model.prompt.apply(reply)
            for _ in 0..<Int(fps) { model.audio.simulate(seconds: 1 / fps); stage.step(dt: 1 / fps) }
        }
        if let h = stage.things.first(where: { $0.kind == .bird && $0.key != stage.selection }) { stage.hover = h.key }

        let scale: Float = 2
        let sceneSize = SIMD2(Float(Popover.width), Float(Popover.sceneHeight))
        stage.sceneSize = sceneSize
        stage.updateLabels()
        let spectrogramOpen = args.contains("--spectrogram")
        let specSize = SIMD2(Float(Popover.stripWidth), Float(spectrogramOpen ? Popover.spectrogramOpen : Popover.spectrogramClosed))
        let sceneRenderer = SceneRenderer(stage: stage)
        let specRenderer = SpectrogramRenderer(stage: stage)
        let tracksRenderer = TracksRenderer(stage: stage)
        // --expand <group>: that group's drawer is open; --more: every lever is shown.
        let controls = stage.controls
        let expand = value("--expand") ?? "none"
        let drawer = controls.macros.first { $0.key == expand || $0.key.hasPrefix(expand + ".") }?.id
        let showAll = args.contains("--more")
        let levers = LeverPanel.visible(controls.macros, flying: model.audio.flying, showAll: showAll)
        let rows = LeverPanel.layout(levers)
        let tracksSize = SIMD2(Float(Popover.sliderWidth), Float(Popover.leverHeight) * Float(max(levers.count, 1)))
        let hover: SIMD2<Float>? = spectrogramOpen ? SIMD2<Float>(specSize.x * 0.72, specSize.y * 0.38) : nil
        specRenderer.hover = hover
        guard let scene = GPU.shared.image(width: Int(sceneSize.x * scale), height: Int(sceneSize.y * scale), encode: {
                  sceneRenderer.encode($0, target: $1, size: sceneSize, scale: scale)
              }),
              let spec = GPU.shared.image(width: Int(specSize.x * scale), height: Int(specSize.y * scale), encode: {
                  specRenderer.encode($0, target: $1, size: specSize, scale: scale)
              }),
              let tracks = GPU.shared.image(width: Int(tracksSize.x * scale), height: Int(tracksSize.y * scale), encode: {
                  tracksRenderer.encode($0, target: $1, size: tracksSize, scale: scale, rows: rows)
              })
        else { return }

        let stills = StillFrames(scene: scene, spectrogram: spec, tracks: tracks, spectrogramHover: hover,
                                 drawer: drawer, showAll: showAll, spectrogramOpen: spectrogramOpen)
        let renderer = ImageRenderer(content: Popover(model: model, stills: stills))
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
