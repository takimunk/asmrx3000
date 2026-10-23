import AppKit
import CoreMotion
import SwiftUI

/// The steps and everything they watch. Head samples are read on the main thread at UI rate
/// (HeadTracker publishes them at <= 30 Hz); nothing here touches the audio path.
@MainActor
final class OnboardingFlow: ObservableObject {
    enum Step: Equatable {
        /// The first step: English or Russian (applies at once; changeable later in settings).
        case language
        /// The head turns by itself; "Разрешить" or "Без трекинга".
        case intro
        /// Permission asked, waiting for the first samples.
        case waiting
        /// Samples arrive: the head follows the listener.
        case linked
        /// Permission granted (or pending) but no samples: no AirPods in the ears.
        case noAirPods
        case denied
        /// "Look at the centre of the screen".
        case centre
        /// The screen's distance and angle.
        case screen
        /// The accent, the API keys and the microphone; all optional. Then into the panel.
        case keys
    }

    @Published private(set) var step: Step = .language
    @Published private(set) var calibrated = false
    @Published private(set) var skippedHead = false
    @Published private(set) var geometry: ScreenGeometry?
    /// The centre step lost the head (AirPods out, sensor stale).
    @Published private(set) var signalLost = false

    let head: HeadTracker
    private let audio: AudioEngine
    private let screens: ScreenWatcher
    weak var window: NSWindow?
    var onFinish: ((Bool) -> Void)?

    // Read by the canvases every frame (not published: they redraw anyway).
    private(set) var stepStart = Date()
    private(set) var linkTime: Date?
    private(set) var calibratedTime: Date?
    /// Seconds the head has been still in the centre step, 0...1 of the hold.
    private(set) var stillness = 0.0
    /// Angular speed of the head, rad/s, smoothed.
    private(set) var rate = 0.0
    private(set) var real = HeadPose()
    let started = Date()

    private var timer: Timer?
    private var requestTime: Date?
    private var lastTick = Date()
    private var lastSample: TimeInterval = 0
    private var lastAuthCheck = Date.distantPast

    static let holdSeconds = 1.0
    static let stillRate = 0.14 // rad/s, about 8 deg/s

    init(head: HeadTracker, audio: AudioEngine, screens: ScreenWatcher) {
        self.head = head
        self.audio = audio
        self.screens = screens
        timer = Timer(timeInterval: 1.0 / 30, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        RunLoop.main.add(timer!, forMode: .common)
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    /// Fresh samples within the last 0.6 s.
    var live: Bool {
        head.enabled && head.active && ProcessInfo.processInfo.systemUptime - head.lastSampleTime < 0.6
    }

    private func go(_ s: Step) {
        guard s != step else { return }
        step = s
        stepStart = Date()
        if s == .centre {
            stillness = 0
            centreWindow()
        }
        if s == .screen { measureScreen() }
    }

    // MARK: Actions

    func allow() {
        requestTime = Date()
        go(.waiting)
        head.requestAccess()
    }

    /// "Без airpods": from the intro this also turns head tracking off, so the app does not
    /// ask for motion access later; it comes back with the "Голова" toggle. Nothing to
    /// calibrate without a head, but the screen's distance and angle still place the sound:
    /// on to the screen step.
    func continueWithoutHead() {
        skippedHead = true
        if step == .intro, CMHeadphoneMotionManager.authorizationStatus() == .notDetermined {
            head.enabled = false
        }
        go(.screen)
    }

    func toIntro() { go(.intro) }

    func toCentre() { go(.centre) }

    func skipCentre() { go(.screen) }

    func toKeys() { go(.keys) }

    /// Development: `--onboarding-step centre|screen|keys` opens at that step, and
    /// `--onboarding-finish <seconds>` presses "слушать" after that long (to watch the glide).
    func applyDevArguments() {
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--onboarding-step"), i + 1 < args.count {
            switch args[i + 1] {
            case "intro": go(.intro)
            case "centre": go(.centre)
            case "screen": go(.screen)
            case "keys": go(.keys)
            default: break
            }
        }
        if let i = args.firstIndex(of: "--onboarding-finish"), i + 1 < args.count, let t = Double(args[i + 1]) {
            DispatchQueue.main.asyncAfter(deadline: .now() + t) { [weak self] in self?.finish(play: true) }
        }
    }

    func setDistance(_ d: Double) {
        guard var g = geometry else { return }
        ScreenGeometry.setDistance(d, builtIn: g.builtIn)
        g.distance = ScreenGeometry.distance(builtIn: g.builtIn)
        geometry = g
        if let screen = window?.screen { screens.apply(screen) } else { screens.apply() }
    }

    /// Degrees of the screen centre from the eyes, − = below.
    func setElevation(_ e: Double) {
        guard var g = geometry else { return }
        ScreenGeometry.setElevation(e, builtIn: g.builtIn)
        g.elevation = ScreenGeometry.elevation(builtIn: g.builtIn)
        geometry = g
        if let screen = window?.screen { screens.apply(screen) } else { screens.apply() }
    }

    func finish(play: Bool) {
        Onboarding.done = true
        stop()
        onFinish?(play)
    }

    /// The close mark or esc: done, without playback.
    func dismiss() { finish(play: false) }

    // MARK: Tick (30 Hz, main thread)

    private func tick() {
        let now = Date()
        let dt = min(now.timeIntervalSince(lastTick), 0.1)
        lastTick = now
        let inStep = now.timeIntervalSince(stepStart)

        // Head speed from the UI samples.
        if head.lastSampleTime != lastSample, head.active {
            let gap = max(head.lastSampleTime - lastSample, 1.0 / 60)
            let next = HeadPose(yaw: head.yaw, pitch: head.pitch, roll: head.roll)
            if lastSample > 0, gap < 0.3 {
                var dy = next.yaw - real.yaw
                if dy > .pi { dy -= 2 * .pi } else if dy < -.pi { dy += 2 * .pi }
                let r = (dy * dy + pow(next.pitch - real.pitch, 2)).squareRoot() / gap
                rate += (r - rate) * 0.35
            }
            real = next
            lastSample = head.lastSampleTime
        }

        switch step {
        case .waiting, .noAirPods, .denied:
            if live {
                linkTime = now
                head.recenter() // the head on screen starts out facing you
                go(.linked)
                return
            }
            if now.timeIntervalSince(lastAuthCheck) > 0.5 {
                lastAuthCheck = now
                let status = CMHeadphoneMotionManager.authorizationStatus()
                if head.denied || status == .denied || status == .restricted {
                    go(.denied)
                    return
                }
                // No headphones at all: say so almost at once. Connected but silent (not in
                // the ears, no head tracking): give them a few seconds.
                let patience: Double = !head.connected ? 1.5 : (status == .authorized ? 3 : 8)
                // (Not while macOS is still asking for access.)
                if step == .waiting, status != .notDetermined, status == .authorized || !head.connected,
                   let t = requestTime, now.timeIntervalSince(t) > patience {
                    go(.noAirPods)
                }
            }
        case .centre:
            signalLost = !live && inStep > 0.5
            if calibrated { break }
            let facing = abs(real.pitch) < 0.5
            if live, inStep > 1.2, rate < Self.stillRate, facing {
                stillness = min(1, stillness + dt / Self.holdSeconds)
            } else {
                stillness = max(0, stillness - dt * 2.5)
            }
            if stillness >= 1 { calibrate() }
        default:
            break
        }
    }

    private func calibrate() {
        calibrated = true
        calibratedTime = Date()
        head.recenter()
        measureScreen()
        Chime.play()
        NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) { [weak self] in
            guard let self, self.step == .centre else { return }
            self.go(.screen)
        }
    }

    /// The display under the window is the one the listener faces.
    private func measureScreen() {
        let screen = window?.screen ?? ScreenGeometry.preferredScreen
        if let screen { ScreenGeometry.remember(screen) }
        geometry = screens.apply(screen)
    }

    /// Puts the window's centre (where the target is drawn) on the screen's physical centre,
    /// with the same glide that later carries it under the menu bar.
    private func centreWindow() {
        guard let window, let screen = window.screen ?? NSScreen.main else { return }
        let size = window.frame.size
        let f = screen.frame
        let frame = NSRect(x: f.midX - size.width / 2, y: f.midY - size.height / 2, width: size.width, height: size.height)
        guard frame.integral != window.frame.integral else { return }
        if Motion.reduced {
            window.setFrame(frame, display: true)
        } else {
            Glide.run(window, to: frame, duration: Glide.duration(from: window.frame, to: frame))
        }
    }
}

/// Owns the onboarding window. One at a time; closing it counts as finished (without playing).
///
/// The window is drawn like the menu bar panel (borderless, the panel's ground, the same 12 pt
/// continuous corners and shadow), so the two read as one object: at the end the window's
/// content fades, the window glides and grows into the panel's frame under the icon, and the
/// panel's content fades in there (MenuBarController.open(fadeIn:)). Re-run from the panel, it
/// grows out of the panel's frame the same way. The calibration step's move to the screen's
/// centre is the same glide. Reduce Motion: no glide, a short cross-fade in place.
@MainActor
final class OnboardingController: NSObject, NSWindowDelegate {
    private weak var model: AppModel?
    private var window: OnboardingWindow?
    private var host: NSView?
    private var flow: OnboardingFlow?

    static let size = NSSize(width: 460, height: 404)
    static let corner: CGFloat = 12

    init(model: AppModel) {
        self.model = model
    }

    func show() {
        guard let model else { return }
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        // Re-run from the panel: grow out of it.
        let from = MenuBarController.shared?.openFrame
        MenuBarController.shared?.close()

        let flow = OnboardingFlow(head: model.head, audio: model.audio, screens: model.screens)
        let screen = NSScreen.main ?? NSScreen.screens.first
        let centred: NSRect = {
            guard let f = screen?.frame else { return NSRect(origin: .zero, size: Self.size) }
            return NSRect(x: (f.midX - Self.size.width / 2).rounded(), y: (f.midY - Self.size.height / 2).rounded(),
                          width: Self.size.width, height: Self.size.height)
        }()
        let start = (from != nil && !Motion.reduced) ? from! : centred

        let w = OnboardingWindow(contentRect: start, styleMask: [.borderless], backing: .buffered, defer: false)
        w.title = "ASMR X3000"
        w.isOpaque = false
        w.backgroundColor = .clear
        w.hasShadow = true
        // Moved only from the art's background (WindowDragArea): "movable by background" would
        // also take drags meant for the screen slab in the side view.
        w.isMovableByWindowBackground = false
        w.isReleasedWhenClosed = false
        w.appearance = NSAppearance(named: .darkAqua)
        w.collectionBehavior = [.fullScreenAuxiliary]
        w.delegate = self
        w.onCancel = { [weak flow] in flow?.dismiss() }

        // The ground: a rounded layer like the panel's; the SwiftUI content sits on it at its
        // fixed size, pinned to the top and centred, so the window can change size under it.
        let ground = NSView(frame: NSRect(origin: .zero, size: start.size))
        ground.wantsLayer = true
        ground.layer?.backgroundColor = NSColor(Ink.panel).cgColor
        ground.layer?.cornerRadius = Self.corner
        ground.layer?.cornerCurve = .continuous
        ground.layer?.masksToBounds = true
        let hosting = NSHostingView(rootView: OnboardingView(flow: flow))
        hosting.sizingOptions = []
        hosting.frame = NSRect(x: ((start.width - Self.size.width) / 2).rounded(), y: start.height - Self.size.height,
                               width: Self.size.width, height: Self.size.height)
        hosting.autoresizingMask = [.minXMargin, .maxXMargin, .minYMargin]
        ground.addSubview(hosting)
        w.contentView = ground

        flow.window = w
        flow.onFinish = { [weak self] play in self?.close(play: play) }
        self.flow = flow
        self.host = hosting
        window = w
        NSApp.activate(ignoringOtherApps: true)
        flow.applyDevArguments()
        if start != centred {
            hosting.alphaValue = 0
            w.makeKeyAndOrderFront(nil)
            let d = Glide.duration(from: start, to: centred)
            Glide.run(w, to: centred, duration: d)
            DispatchQueue.main.asyncAfter(deadline: .now() + d * 0.55) {
                NSAnimationContext.runAnimationGroup { ctx in
                    ctx.duration = d * 0.45
                    hosting.animator().alphaValue = 1
                }
            }
        } else {
            w.makeKeyAndOrderFront(nil)
        }
    }

    private func close(play: Bool) {
        guard let w = window else { return }
        let hosting = host
        window = nil
        flow = nil
        host = nil
        w.delegate = nil
        w.ignoresMouseEvents = true
        guard play, let model else {
            fadeOut(w)
            return
        }
        model.audio.place = .home
        model.audio.play()
        guard let menu = MenuBarController.shared else {
            fadeOut(w)
            model.openPanel()
            return
        }
        let target = menu.landingFrame(preload: !Motion.reduced)
        if Motion.reduced {
            // A cross-fade in place: the window goes, the panel comes.
            fadeOut(w)
            menu.open(fadeIn: 0.2)
            return
        }
        // Into the panel: the words fade, the ground glides and grows into the panel's frame
        // under the icon, and the panel's content fades in there.
        w.level = .popUpMenu
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.16
            hosting?.animator().alphaValue = 0
        }
        // The sections start settling in just before the window lands (it is a few points
        // away by then), over the window's own ground; on landing the panel takes over.
        var arriving = false
        Glide.run(w, to: { menu.placedFrame ?? target }, duration: Glide.duration(from: w.frame, to: target),
                  progress: { u in
                      if u >= 0.78, !arriving { arriving = true; menu.beginArrival() }
                  }) {
            menu.land()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                w.orderOut(nil)
                w.contentView = nil
            }
        }
    }

    private func fadeOut(_ w: NSWindow) {
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = Motion.reduced ? 0.12 : 0.18
            w.animator().alphaValue = 0
        }, completionHandler: {
            MainActor.assumeIsolated {
                w.orderOut(nil)
                w.contentView = nil
            }
        })
    }

    func windowWillClose(_ notification: Notification) {
        guard window != nil else { return }
        Onboarding.done = true
        flow?.stop()
        window = nil
        flow = nil
        host = nil
    }
}

/// Borderless, but takes the keyboard (the buttons' keys, the key fields); esc closes it.
final class OnboardingWindow: NSWindow {
    var onCancel: (() -> Void)?
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
    override func cancelOperation(_ sender: Any?) { onCancel?() }
}

/// Moves and resizes a window along one eased curve (the onboarding's moves: to the screen's
/// centre, under the menu bar, out of the panel). Frame steps on the display's refresh via a
/// timer; a newer glide on the same window replaces the running one.
@MainActor
enum Glide {
    private static var running: [ObjectIdentifier: Timer] = [:]

    /// Longer for longer trips, within 0.42...0.7 s.
    static func duration(from a: NSRect, to b: NSRect) -> Double {
        let d = hypot(b.midX - a.midX, b.midY - a.midY) + abs(b.width - a.width) + abs(b.height - a.height)
        return min(0.7, max(0.42, 0.36 + Double(d) / 2400))
    }

    static func run(_ window: NSWindow, to target: NSRect, duration: Double, completion: (() -> Void)? = nil) {
        run(window, to: { target }, duration: duration, completion: completion)
    }

    /// `target` is read every frame: the glide lands where the destination is by then.
    /// `progress` gets the glide's time, 0...1, every frame.
    static func run(_ window: NSWindow, to targetNow: @escaping () -> NSRect, duration: Double,
                    progress: ((Double) -> Void)? = nil, completion: (() -> Void)? = nil) {
        let id = ObjectIdentifier(window)
        running[id]?.invalidate()
        let from = window.frame
        let start = CACurrentMediaTime()
        let timer = Timer(timeInterval: 1.0 / 120, repeats: true) { timer in
            MainActor.assumeIsolated {
                let u = min(1, (CACurrentMediaTime() - start) / max(duration, 0.01))
                let k = CGFloat(ease(u))
                let target = targetNow()
                let f = NSRect(x: from.minX + (target.minX - from.minX) * k,
                               y: from.minY + (target.minY - from.minY) * k,
                               width: from.width + (target.width - from.width) * k,
                               height: from.height + (target.height - from.height) * k)
                window.setFrame(u >= 1 ? target : f.integral, display: true)
                window.invalidateShadow()
                progress?(u)
                if u >= 1 {
                    timer.invalidate()
                    running[id] = nil
                    window.invalidateShadow()
                    completion?()
                }
            }
        }
        running[id] = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    /// A soft ease-in-out with a long settle: cubic Bézier (0.4, 0, 0.2, 1), quick to leave,
    /// slow to arrive.
    static func ease(_ u: Double) -> Double {
        let x = min(max(u, 0), 1)
        let x1 = 0.4, x2 = 0.2, y1 = 0.0, y2 = 1.0
        func bez(_ t: Double, _ a: Double, _ b: Double) -> Double {
            3 * (1 - t) * (1 - t) * t * a + 3 * (1 - t) * t * t * b + t * t * t
        }
        // Solve x(t) = x by bisection (monotonic in t), then y(t).
        var lo = 0.0, hi = 1.0, t = x
        for _ in 0..<24 {
            t = (lo + hi) / 2
            if bez(t, x1, x2) < x { lo = t } else { hi = t }
        }
        return bez(t, y1, y2)
    }
}

/// The calibration's "done": two soft sine bells, E6 then B6, synthesised (no asset).
enum Chime {
    private static var sound: NSSound?

    static func play() {
        sound = NSSound(data: wav())
        sound?.volume = 0.5
        sound?.play()
    }

    private static func wav() -> Data {
        let rate = 44_100, seconds = 0.9
        let n = Int(Double(rate) * seconds)
        var pcm = [Int16](repeating: 0, count: n)
        for (start, f, amp) in [(0.0, 1318.5, 0.32), (0.09, 1975.5, 0.26)] {
            for i in 0..<n {
                let t = Double(i) / Double(rate) - start
                guard t >= 0 else { continue }
                let env = min(t / 0.004, 1) * exp(-t * 5.5)
                let v = amp * env * (sin(2 * .pi * f * t) + 0.18 * sin(2 * .pi * f * 2.76 * t) * exp(-t * 9))
                pcm[i] = Int16(clamping: Int(Double(pcm[i]) + v * 32_767))
            }
        }
        var d = Data()
        func put<T: FixedWidthInteger>(_ v: T) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
        d.append(contentsOf: Array("RIFF".utf8)); put(UInt32(36 + n * 2))
        d.append(contentsOf: Array("WAVEfmt ".utf8)); put(UInt32(16)); put(UInt16(1)); put(UInt16(1))
        put(UInt32(rate)); put(UInt32(rate * 2)); put(UInt16(2)); put(UInt16(16))
        d.append(contentsOf: Array("data".utf8)); put(UInt32(n * 2))
        pcm.withUnsafeBytes { d.append(contentsOf: $0) }
        return d
    }
}
