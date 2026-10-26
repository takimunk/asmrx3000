import AppKit
import MetalKit
import SwiftUI

/// MTKView that renders only while its window is actually visible, so the popover costs
/// nothing once closed. The scene view owns the display link; other stage views (the
/// spectrogram) are drawn right after it in the same frame.
class StageView: MTKView, MTKViewDelegate {
    let stage: Stage
    private var observers: [NSObjectProtocol] = []
    /// False for views drawn by the scene view's frame instead of their own display link.
    var drivesItself: Bool { true }
    private var followerFrame = 0

    /// 30 fps at rest; 60 while the pointer or keyboard is at work (see Stage.poke).
    static let restFPS = 30
    static let activeFPS = 60
    static let fixedFPS = Int(ProcessInfo.processInfo.environment["ASMR_FPS"] ?? "")

    init(stage: Stage) {
        self.stage = stage
        super.init(frame: .zero, device: GPU.shared.device)
        colorPixelFormat = .bgra8Unorm
        colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        framebufferOnly = true
        preferredFramesPerSecond = Self.fixedFPS ?? Self.restFPS
        clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
        delegate = self
        isPaused = true
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        guard window != nil else {
            isPaused = true
            if drivesItself, shown { shown = false; stage.setLive(false) }
            return
        }
        if drivesItself, let w = window {
            Self.debug("moved to \(w.className) visible=\(w.isVisible) level=\(w.level.rawValue) view=\(Int(bounds.width))×\(Int(bounds.height)) paused=\(isPaused)")
        }
        // The display link runs whenever the view is in a window; `draw` itself checks every
        // frame whether the window is on screen. Window notifications (occlusion, key) are not
        // reliable for the menu bar panel on newer macOS: it could stay paused while showing.
        isPaused = !drivesItself
    }

    /// Whether the window was on screen at the last frame (drives `Stage.setLive`).
    private var shown = false
    private var loggedFrame = false
    private var traceFrame = 0

    /// Called at the top of every frame: false while the window is ordered out or fully hidden.
    private func checkShown() -> Bool {
        traceFrame &+= 1
        if traceFrame % 60 == 1, let w = window {
            Self.debug("frame \(traceFrame) \(w.className) visible=\(w.isVisible) occl=\(w.occlusionState.rawValue) key=\(w.isKeyWindow) view=\(Int(bounds.width))×\(Int(bounds.height)) hidden=\(isHiddenOrHasHiddenAncestor) alpha=\(w.alphaValue)")
        }
        let now = (window?.isVisible ?? false) && (window?.occlusionState.contains(.visible) ?? false || Self.ignoreOcclusion)
        if now != shown {
            shown = now
            stage.setLive(now)
            Self.debug("stage \(now ? "shown" : "hidden") in \(window.map { String(describing: type(of: $0)) } ?? "-")")
            if now { windowBecameVisible() }
        }
        return now
    }

    /// The menu bar panel reports itself occluded while on screen on macOS 26+.
    private static var ignoreOcclusion: Bool {
        if #available(macOS 26, *) { return true }
        return false
    }

    /// Development trace, appended to ~/Library/Logs/ASMRX3000.log when ASMR_DEBUG is set.
    static func debug(_ s: String) {
        guard ProcessInfo.processInfo.environment["ASMR_DEBUG"] != nil || CommandLine.arguments.contains("--debug-log") else { return }
        let url = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/ASMRX3000.log")
        let line = "\(Date()) \(s)\n"
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(line.data(using: .utf8)!); try? h.close()
        } else {
            try? line.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    func windowBecameVisible() {}

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        if drivesItself {
            guard checkShown() else { return }
            stage.tick()
            let fps = Self.fixedFPS ?? (CACurrentMediaTime() < stage.activeUntil ? Self.activeFPS : Self.restFPS)
            if preferredFramesPerSecond != fps { preferredFramesPerSecond = fps }
        }
        guard let drawable = currentDrawable, let cb = GPU.shared.queue.makeCommandBuffer() else {
            if drivesItself { Self.debug("no drawable (\(Int(bounds.width))×\(Int(bounds.height)))") }
            return
        }
        if drivesItself && !loggedFrame {
            loggedFrame = true
            Self.debug("first frame \(Int(bounds.width))×\(Int(bounds.height))")
            cb.addCompletedHandler { b in if let e = b.error { StageView.debug("frame error: \(e)") } }
        }
        let scale = Float(window?.backingScaleFactor ?? 2)
        render(cb, target: drawable.texture, size: SIMD2(Float(bounds.width), Float(bounds.height)), scale: scale)
        cb.present(drawable)
        cb.commit()
        // The spectrogram and tracks follow at half rate: they scroll slowly and cost a drawable each.
        followerFrame &+= 1
        if drivesItself && (preferredFramesPerSecond <= Self.restFPS || followerFrame % 2 == 0) {
            for follower in stage.followers.allObjects where follower.window === window { follower.draw() }
        }
    }

    func render(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {}

    override var isFlipped: Bool { true }

    func local(_ event: NSEvent) -> SIMD2<Float> {
        let p = convert(event.locationInWindow, from: nil)
        return SIMD2(Float(p.x), Float(p.y))
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self))
    }
}

// MARK: - Scene

final class SceneStageView: StageView {
    private lazy var renderer = SceneRenderer(stage: stage)
    private var downAt: SIMD2<Float>?
    private var dragged = false

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func windowBecameVisible() {
        stage.focusScene = { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self)
        }
        // MenuBarExtra windows are key while open; take the keyboard so arrows reach us.
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window else { return }
            if !(window.firstResponder is NSTextView) { window.makeFirstResponder(self) }
        }
    }

    override func render(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {
        stage.sceneSize = size
        renderer.encode(cb, target: target, size: size, scale: scale)
    }

    private var size: SIMD2<Float> { SIMD2(Float(bounds.width), Float(bounds.height)) }

    override func mouseDown(with event: NSEvent) {
        stage.poke()
        window?.makeFirstResponder(self)
        downAt = local(event)
        dragged = false
    }

    override func mouseDragged(with event: NSEvent) {
        stage.poke()
        guard let start = downAt else { return }
        let p = local(event)
        if !dragged && simd_distance(p, start) < 3 { return }
        dragged = true
        stage.camera.azimuth -= Float(event.deltaX) * 0.008
        stage.camera.elevation = min(max(stage.camera.elevation + Float(event.deltaY) * 0.006, 0.12), 1.35)
        NSCursor.closedHand.set()
    }

    override func mouseUp(with event: NSEvent) {
        stage.poke()
        defer { downAt = nil; NSCursor.arrow.set() }
        guard !dragged else { return }
        let key = stage.pick(at: local(event), size: size)
        stage.select(key)
        stage.hover = key
    }

    override func mouseMoved(with event: NSEvent) {
        stage.poke()
        stage.hover = stage.pick(at: local(event), size: size)
        (stage.hover != nil ? NSCursor.pointingHand : NSCursor.arrow).set()
    }

    override func mouseExited(with event: NSEvent) {
        stage.hover = nil
        NSCursor.arrow.set()
    }

    override func scrollWheel(with event: NSEvent) {
        stage.poke()
        let d = Float(event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * 8)
        stage.camera.distance = min(max(stage.camera.distance * exp(-d * 0.01), 9), 40)
    }

    override func magnify(with event: NSEvent) {
        stage.poke()
        stage.camera.distance = min(max(stage.camera.distance * Float(1 - event.magnification), 9), 40)
    }

    override func keyDown(with event: NSEvent) {
        stage.poke()
        if !Keys.handle(event, stage: stage) { super.keyDown(with: event) }
    }

}

/// Keyboard mapping for the scene (documented in README):
///   ← →        move left / right on screen (along the ground)
///   ↑ ↓        move away from / towards the camera (along the ground)
///   ⇧↑ ⇧↓      raise / lower
///   ⌥ + any    fine steps (5 cm instead of 25 cm)
///   tab        select the next object (⇧tab previous), esc releases the selection (then closes a drawer)
///   space      play / pause
///   H          head tracking on / off       C   recenter
///   return, /  write to the prompt
enum Keys {
    static func handle(_ event: NSEvent, stage: Stage) -> Bool {
        let fine = event.modifierFlags.contains(.option)
        let shift = event.modifierFlags.contains(.shift)
        let step: Float = fine ? 0.05 : 0.25
        switch event.keyCode {
        case 123: stage.nudge(SIMD3(-step, 0, 0))
        case 124: stage.nudge(SIMD3(step, 0, 0))
        case 126: stage.nudge(shift ? SIMD3(0, 0, step) : SIMD3(0, step, 0))
        case 125: stage.nudge(shift ? SIMD3(0, 0, -step) : SIMD3(0, -step, 0))
        case 48: stage.cycleSelection(backwards: shift)
        case 53:
            // Esc lets go of the selection; with nothing selected it closes an open drawer.
            if stage.selection != nil { stage.select(nil) } else if stage.controls.expanded != nil { stage.controls.expanded = nil } else { return false }
        case 49: stage.audio.isPlaying ? stage.audio.pause() : stage.audio.play()
        // Return and / write to the prompt; a clip scene has none.
        case 36 where !stage.isClipScene, 76 where !stage.isClipScene: stage.focusPrompt?()
        default:
            guard event.modifierFlags.intersection([.command, .control, .option]).isEmpty else { return false }
            let key = event.charactersIgnoringModifiers?.lowercased()
            switch key {
            case "h" where HeadTracker.supported: stage.head.enabled.toggle()
            case "c" where HeadTracker.supported: stage.head.recenter()
            case "/" where !stage.isClipScene: stage.focusPrompt?()
            default: return false
            }
        }
        return true
    }
}

// MARK: - Spectrogram

final class SpectrogramStageView: StageView {
    private lazy var renderer = SpectrogramRenderer(stage: stage)
    var onHover: ((SIMD2<Float>?) -> Void)?

    override var drivesItself: Bool { false }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        stage.followers.add(self)
    }

    override func render(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {
        renderer.encode(cb, target: target, size: size, scale: scale)
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseMoved(with event: NSEvent) {
        stage.poke()
        renderer.hover = local(event)
        onHover?(renderer.hover)
    }

    override func mouseExited(with event: NSEvent) {
        renderer.hover = nil
        onHover?(nil)
    }

    override func mouseUp(with event: NSEvent) {
        stage.poke()
        if event.clickCount == 2 {
            stage.zoom = SIMD2(0, 1)
        } else {
            stage.frozen.toggle()
        }
        onHover?(renderer.hover)
    }

    /// Scroll zooms the frequency axis around the pointer.
    override func scrollWheel(with event: NSEvent) {
        stage.poke()
        let d = Float(event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * 8)
        let y = 1 - local(event).y / Float(bounds.height)
        let z = stage.zoom
        let pivot = z.x + (z.y - z.x) * y
        let span = min(max((z.y - z.x) * exp(-d * 0.01), 0.08), 1)
        var lo = pivot - span * y
        lo = min(max(lo, 0), 1 - span)
        stage.zoom = SIMD2(lo, lo + span)
        onHover?(renderer.hover)
    }
}

// MARK: - Tracks

/// The live waveforms behind the sliders. Input goes to the SwiftUI controls above it.
final class TracksStageView: StageView {
    private lazy var renderer = TracksRenderer(stage: stage)
    var rows: [ScopeRow] = []

    override var drivesItself: Bool { false }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        stage.followers.add(self)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func render(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {
        renderer.encode(cb, target: target, size: size, scale: scale, rows: rows)
    }
}

struct TracksSurface: NSViewRepresentable {
    let stage: Stage
    var rows: [ScopeRow]
    func makeNSView(context: Context) -> TracksStageView {
        let v = TracksStageView(stage: stage)
        v.rows = rows
        return v
    }
    func updateNSView(_ view: TracksStageView, context: Context) { view.rows = rows }
}

// MARK: - SwiftUI

struct SceneSurface: NSViewRepresentable {
    let stage: Stage
    func makeNSView(context: Context) -> SceneStageView { SceneStageView(stage: stage) }
    func updateNSView(_ view: SceneStageView, context: Context) {}
}

struct SpectrogramSurface: NSViewRepresentable {
    let stage: Stage
    var onHover: (SIMD2<Float>?) -> Void
    func makeNSView(context: Context) -> SpectrogramStageView {
        let v = SpectrogramStageView(stage: stage)
        v.onHover = onHover
        return v
    }
    func updateNSView(_ view: SpectrogramStageView, context: Context) { view.onHover = onHover }
}
