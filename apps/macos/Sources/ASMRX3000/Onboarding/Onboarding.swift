import AppKit
import CoreMotion
import SwiftUI

/// First launch: why we want the AirPods' motion, the permission, the head that follows yours,
/// "look at the centre of the screen" (sets the audio's forward), the screen's size and
/// distance, then the forest starts. Shown until finished once ("onboarding.done"); again with
/// `--onboarding` or from the popover (right click on "Голова" or on recenter).
enum Onboarding {
    static let doneKey = "onboarding.done"
    static let forced = CommandLine.arguments.contains("--onboarding")
    /// Command-line runs that must never open a window or ask for permissions.
    static let headless = ["--snapshot", "--bench", "--prompt-live", "--voice-check", "--menubar-frames"]
        .contains { CommandLine.arguments.contains($0) }

    static var done: Bool {
        get { UserDefaults.standard.bool(forKey: doneKey) }
        set { UserDefaults.standard.set(newValue, forKey: doneKey) }
    }

    /// Should run at this launch (HeadTracker holds back its permission request until then).
    static var pending: Bool { !headless && (forced || !done) }
}

/// The steps and everything they watch. Head samples are read on the main thread at UI rate
/// (HeadTracker publishes them at <= 30 Hz); nothing here touches the audio path.
@MainActor
final class OnboardingFlow: ObservableObject {
    enum Step: Equatable {
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
        case done
    }

    @Published private(set) var step: Step = .intro
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
        if s == .done { measureScreen() }
    }

    // MARK: Actions

    func allow() {
        requestTime = Date()
        go(.waiting)
        head.requestAccess()
    }

    /// "Без airpods": from the intro this also turns head tracking off, so the app does not
    /// ask for motion access later; it comes back with the "Голова" toggle. Nothing to
    /// calibrate without a head: straight into the app (the screen keeps its defaults).
    func continueWithoutHead() {
        skippedHead = true
        if step == .intro, CMHeadphoneMotionManager.authorizationStatus() == .notDetermined {
            head.enabled = false
        }
        finish(play: true)
    }

    func toCentre() { go(.centre) }

    func skipCentre() { go(.done) }

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
            self.go(.done)
        }
    }

    /// The display under the window is the one the listener faces.
    private func measureScreen() {
        let screen = window?.screen ?? ScreenGeometry.preferredScreen
        if let screen { ScreenGeometry.remember(screen) }
        geometry = screens.apply(screen)
    }

    /// Puts the window's centre (where the target is drawn) on the screen's physical centre.
    private func centreWindow() {
        guard let window, let screen = window.screen ?? NSScreen.main else { return }
        let size = window.frame.size
        let f = screen.frame
        let frame = NSRect(x: f.midX - size.width / 2, y: f.midY - size.height / 2, width: size.width, height: size.height)
        window.setFrame(frame, display: true, animate: !Motion.reduced)
    }
}

/// Owns the onboarding window. One at a time; closing it counts as finished (without playing).
@MainActor
final class OnboardingController: NSObject, NSWindowDelegate {
    private weak var model: AppModel?
    private var window: NSWindow?
    private var flow: OnboardingFlow?

    static let size = NSSize(width: 460, height: 404)

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
        let flow = OnboardingFlow(head: model.head, audio: model.audio, screens: model.screens)
        let w = NSWindow(contentRect: NSRect(origin: .zero, size: Self.size),
                         styleMask: [.titled, .closable, .fullSizeContentView], backing: .buffered, defer: false)
        w.titlebarAppearsTransparent = true
        w.titleVisibility = .hidden
        w.title = "ASMR X3000"
        w.isMovableByWindowBackground = true
        w.isReleasedWhenClosed = false
        w.backgroundColor = NSColor(Ink.panel)
        w.appearance = NSAppearance(named: .darkAqua)
        w.delegate = self
        w.contentView = NSHostingView(rootView: OnboardingView(flow: flow))
        if let screen = NSScreen.main ?? NSScreen.screens.first {
            let f = screen.frame
            w.setFrameOrigin(NSPoint(x: f.midX - Self.size.width / 2, y: f.midY - Self.size.height / 2))
        }
        flow.window = w
        flow.onFinish = { [weak self] play in self?.close(play: play) }
        self.flow = flow
        window = w
        NSApp.activate(ignoringOtherApps: true)
        w.makeKeyAndOrderFront(nil)
    }

    private func close(play: Bool) {
        let w = window
        window = nil
        flow = nil
        w?.delegate = nil
        w?.close()
        if play, let model {
            model.audio.place = .home
            model.audio.play()
            // Straight into the app: open the menu bar panel once the window has gone.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { model.openPanel() }
        }
    }

    func windowWillClose(_ notification: Notification) {
        // The close button: done, but no surprise playback.
        guard window != nil else { return }
        Onboarding.done = true
        flow?.stop()
        window = nil
        flow = nil
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
