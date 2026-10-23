import AppKit
import SwiftUI

/// The onboarding window, built like the popover so the hand-over into the panel feels like one
/// product: the art full-bleed on top (where the popover has its scene), then the panel's ground,
/// Coppice Mono at the panel's sizes, its greys and its buttons on the 4 pt grid. The art is
/// her portrait through an ordered dither (Dither.swift): paper on the lit side, a neutral
/// screen in the shadow, the accent only for the listener (the earbud, the iris).
struct OnboardingView: View {
    @ObservedObject var flow: OnboardingFlow
    @ObservedObject private var accent = Accent.shared
    @ObservedObject private var lang = Language.shared

    static let artHeight: CGFloat = 256
    /// The last step needs room for its rows: the art shrinks to a band across her eye.
    static let keysArtHeight: CGFloat = 132
    static let steps = 5

    private var artHeight: CGFloat { flow.step == .keys ? Self.keysArtHeight : Self.artHeight }

    var body: some View {
        ZStack(alignment: .topLeading) {
            Ink.panel
            VStack(alignment: .leading, spacing: 0) {
                ArtStage(flow: flow)
                    .frame(width: OnboardingController.size.width, height: artHeight)
                    .background(WindowDragArea())
                    .background(Color.black)
                    .clipped()
                ZStack(alignment: .topLeading) {
                    words
                        .id(wordsKey)
                        .transition(.opacity)
                }
                .padding(.horizontal, Grid.x6)
                .padding(.top, Grid.x4 + 2)
                .padding(.bottom, Grid.x6 - 4)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            if flow.step == .screen, flow.geometry != nil {
                ScreenHandle(flow: flow)
            }
            CloseMark { flow.dismiss() }
                .padding(Grid.x2)
                .frame(maxWidth: .infinity, alignment: .topTrailing)
        }
        .animation(Motion.reduced ? nil : .easeInOut(duration: 0.28), value: wordsKey)
        .animation(Motion.reduced ? nil : .spring(response: 0.5, dampingFraction: 0.9), value: artHeight)
        .coordinateSpace(name: ScreenHandle.space)
        .frame(width: OnboardingController.size.width, height: OnboardingController.size.height)
        .environment(\.colorScheme, .dark)
    }

    /// What the words show; a change cross-fades them.
    private var wordsKey: String {
        switch flow.step {
        case .language: return "language"
        case .intro: return "intro"
        case .waiting: return "waiting"
        case .linked: return "linked"
        case .noAirPods: return "noAirPods"
        case .denied: return "denied"
        case .centre: return "centre"
        case .screen: return "screen"
        case .keys: return "keys"
        }
    }

    private var stepNumber: Int {
        switch flow.step {
        case .language: return 1
        case .intro, .waiting, .linked, .noAirPods, .denied: return 2
        case .centre: return 3
        case .screen: return 4
        case .keys: return 5
        }
    }

    // MARK: Words

    @ViewBuilder private var words: some View {
        switch flow.step {
        case .language:
            Words(title: "language · язык" /* l10n-ok */, step: stepNumber,
                  lines: [tr("pick one. you can change it later in settings.")]) {
                ForEach(Lang.allCases) { l in
                    PanelButton(title: l.native, selected: lang.current == l) { Language.shared.select(l) }
                }
                Spacer(minLength: 0)
                PanelButton(title: tr("next"), primary: true) { flow.toIntro() }
            }
        case .intro:
            Words(title: tr("linking your head to the sound"), step: stepNumber,
                  lines: [tr("airpods know where your head points: the sound stays put when you turn around.")]) {
                PanelButton(title: tr("allow"), primary: true) { flow.allow() }
                PanelButton(title: tr("without airpods")) { flow.continueWithoutHead() }
            }
        case .waiting:
            Words(title: tr("linking your head to the sound"), step: stepNumber,
                  lines: [tr("waiting for a signal from your airpods."), tr("if macos asks for motion access, allow it.")]) {
                PanelButton(title: tr("continue without airpods")) { flow.continueWithoutHead() }
            }
        case .linked:
            Words(title: tr("headphones connected"), step: stepNumber,
                  lines: [tr("turn your head: she follows you.")]) {
                PanelButton(title: tr("next"), primary: true) { flow.toCentre() }
            }
        case .noAirPods:
            Words(title: tr("no airpods found"), step: stepNumber,
                  lines: [tr("connect them: we will carry on as soon as they show up."),
                          tr("or listen without them: the sound plays, it just will not follow your head.")]) {
                PanelButton(title: tr("continue without airpods"), primary: true) { flow.continueWithoutHead() }
            }
        case .denied:
            Words(title: tr("motion access is off"), step: stepNumber,
                  lines: [tr("turn it on in system settings: privacy and security.")]) {
                PanelButton(title: tr("continue without airpods"), primary: true) { flow.continueWithoutHead() }
                PanelButton(title: tr("open settings")) {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        case .centre:
            Words(title: tr("look at the centre"), step: stepNumber,
                  lines: [flow.calibrated ? tr("done: forward is now where the screen is.")
                          : flow.signalLost ? tr("no signal from your airpods. check they are in your ears.")
                          : tr("at the cross. hold still for a second.")]) {
                PanelButton(title: tr("skip")) { flow.skipCentre() }
                    .opacity(flow.calibrated ? 0 : 1)
            }
        case .screen:
            Words(title: tr("one last touch"), step: stepNumber, lines: [tr("set up the screen")]) {
                PanelButton(title: tr("next"), primary: true) { flow.toKeys() }
            } extra: {
                ScreenControls(flow: flow)
            }
        case .keys:
            KeysWords(step: stepNumber) {
                PanelButton(title: tr("listen"), primary: true) { flow.finish(play: true) }
            }
        }
    }
}

/// The step as four short marks: done in graphite, the current one in paper.
private struct StepMarks: View {
    let step: Int
    @ObservedObject private var lang = Language.shared

    var body: some View {
        HStack(spacing: 3) {
            ForEach(1...OnboardingView.steps, id: \.self) { i in
                Capsule()
                    .fill(i == step ? Ink.paper : i < step ? Ink.graphite : Ink.hairline)
                    .frame(width: i == step ? 14 : 8, height: 2)
            }
        }
        .accessibilityElement()
        .accessibilityLabel(Text(tr("Step %d of %d", step, OnboardingView.steps)))
    }
}

/// Title (the popover's prompt size) with the step on the right, plain lines, optional
/// controls, the actions at the bottom.
private struct Words<Actions: View, Extra: View>: View {
    @ObservedObject private var lang = Language.shared
    let title: String
    let step: Int
    let lines: [String]
    @ViewBuilder var actions: Actions
    @ViewBuilder var extra: Extra

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center) {
                Text(title)
                    .font(Ink.text(Ink.Size.prompt + 2))
                    .foregroundStyle(Ink.paper)
                Spacer(minLength: Grid.x2)
                StepMarks(step: step)
            }
            VStack(alignment: .leading, spacing: Grid.x1) {
                ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            }
            .font(Ink.text(Ink.Size.body))
            .lineSpacing(2)
            .foregroundStyle(Ink.graphite)
            .padding(.top, Grid.x2 + 2)
            extra
                .padding(.top, Grid.x3)
            Spacer(minLength: Grid.x2)
            HStack(spacing: Grid.x2) { actions }
        }
    }
}

extension Words where Extra == EmptyView {
    init(title: String, step: Int, lines: [String], @ViewBuilder actions: () -> Actions) {
        self.init(title: title, step: step, lines: lines, actions: actions, extra: { EmptyView() })
    }
}

/// The last step: the accent, the two keys and the microphone, each optional, the same rows
/// as the panel's settings (where they can be changed later).
private struct KeysWords<Actions: View>: View {
    @ObservedObject private var lang = Language.shared
    let step: Int
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center) {
                Text(tr("keys and voice"))
                    .font(Ink.text(Ink.Size.prompt + 2))
                    .foregroundStyle(Ink.paper)
                Spacer(minLength: Grid.x2)
                StepMarks(step: step)
            }
            Text(tr("all optional, and in settings later. typesafe ai turns words into the forest; fal and the microphone are for voice."))
                .font(Ink.text(Ink.Size.body))
                .lineSpacing(2)
                .foregroundStyle(Ink.graphite)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, Grid.x2 + 2)
            VStack(alignment: .leading, spacing: 2) {
                AccentRow(nameWidth: 96)
                KeyRow(account: .typesafe, name: "TypeSafe AI", placeholder: "Add your TypeSafe AI key", nameWidth: 96,
                       help: KeyRow.typesafeHelp)
                KeyRow(account: .fal, name: "fal.ai", placeholder: "Add your fal key", nameWidth: 96, help: KeyRow.falHelp)
                MicrophoneRow(nameWidth: 96)
            }
            .textCase(.lowercase)
            .padding(.top, Grid.x2)
            Spacer(minLength: Grid.x2)
            HStack(spacing: Grid.x2) { actions }
        }
    }
}

/// The popover's button: primary filled with paper, the rest plain until hovered.
private struct PanelButton: View {
    let title: String
    var primary = false
    /// A choice that is currently picked (the language step): outlined in paper.
    var selected = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        let button = Button(action: action) {
            Text(title)
                .font(Ink.text(Ink.Size.title))
                .foregroundStyle(primary ? Ink.panel : hovering || selected ? Ink.paper : Ink.graphite)
                .padding(.horizontal, Grid.x3 + 2)
                .frame(height: 28)
                .background(RoundedRectangle(cornerRadius: Grid.radius, style: .continuous)
                    .fill(primary ? (hovering ? Color(white: 1) : Ink.paper) : hovering ? Ink.raised : .clear))
                .overlay(RoundedRectangle(cornerRadius: Grid.radius, style: .continuous)
                    .strokeBorder(primary ? .clear : selected ? Ink.paper : Ink.hairline, lineWidth: 1))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        if primary {
            button.keyboardShortcut(.defaultAction)
        } else {
            button
        }
    }
}

/// A quiet × over the art's corner: closes the onboarding (counts as done, no playback). Esc too.
private struct CloseMark: View {
    @ObservedObject private var lang = Language.shared
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Canvas { ctx, size in
                var p = Path()
                let r = size.width / 2 - 7
                let c = CGPoint(x: size.width / 2, y: size.height / 2)
                p.move(to: CGPoint(x: c.x - r, y: c.y - r)); p.addLine(to: CGPoint(x: c.x + r, y: c.y + r))
                p.move(to: CGPoint(x: c.x + r, y: c.y - r)); p.addLine(to: CGPoint(x: c.x - r, y: c.y + r))
                ctx.stroke(p, with: .color(hovering ? Ink.paper : Ink.ash), lineWidth: 1.2)
            }
            .frame(width: 22, height: 22)
            .background(Circle().fill(hovering ? Ink.raised : .clear))
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .help(tr("Close"))
        .accessibilityLabel(tr("Close"))
    }
}

/// Drags the borderless window from the art (behind it; the screen handle sits on top).
private struct WindowDragArea: NSViewRepresentable {
    final class DragView: NSView {
        override var mouseDownCanMoveWindow: Bool { true }
        override func mouseDown(with event: NSEvent) { window?.performDrag(with: event) }
    }

    func makeNSView(context: Context) -> NSView { DragView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

/// The screen's distance and angle, each with plain steps and arrow keys.
private struct ScreenControls: View {
    @ObservedObject var flow: OnboardingFlow
    @ObservedObject private var lang = Language.shared

    var body: some View {
        if let g = flow.geometry {
            VStack(alignment: .leading, spacing: Grid.x1) {
                row(tr("to your eyes"), tr("%d cm", Int((g.distance * 100).rounded()))) {
                    Step(label: "−", key: .leftArrow, help: tr("Closer to the screen")) { flow.setDistance(g.distance - 0.05) }
                    Step(label: "+", key: .rightArrow, help: tr("Further from the screen")) { flow.setDistance(g.distance + 0.05) }
                }
                row(tr("angle"), SideLayout.degrees(g.elevation)) {
                    Step(label: "↓", key: .downArrow, help: tr("Screen lower")) { flow.setElevation((g.elevation - 1).rounded()) }
                    Step(label: "↑", key: .upArrow, help: tr("Screen higher")) { flow.setElevation((g.elevation + 1).rounded()) }
                    Text(tr("or drag the screen in the picture")).foregroundStyle(Ink.ash).padding(.leading, Grid.x2)
                }
            }
            .font(Ink.text(Ink.Size.body))
        }
    }

    private func row<V: View>(_ name: String, _ value: String, @ViewBuilder _ steps: () -> V) -> some View {
        HStack(spacing: Grid.x1) {
            Text(name).foregroundStyle(Ink.graphite).frame(width: 64, alignment: .leading)
            Text(value).foregroundStyle(Ink.paper).frame(width: 52, alignment: .leading)
            steps()
        }
    }

    private struct Step: View {
        let label: String
        let key: KeyEquivalent
        let help: String
        let action: () -> Void
        @State private var hovering = false

        var body: some View {
            Button(action: action) {
                Text(label)
                    .font(Ink.text(Ink.Size.title))
                    .foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                    .frame(width: 22, height: 20)
                    .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(hovering ? Ink.raised : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .keyboardShortcut(key, modifiers: [])
            .onHover { hovering = $0 }
            .accessibilityLabel(help)
        }
    }
}

// MARK: - The art

/// Springs and filters that live across frames (a class, so the canvas can step them).
final class ArtAnimator {
    var blend = 0.0, blendV = 0.0
    var real = HeadPose()
    var gaze = SIMD2<Double>(0, 0), gazeV = SIMD2<Double>(0, 0)
    var pod: SIMD2<Double>?
    var podProfile = false
    var ring = 0.0
    var last: Date?

    /// Her pose: her own slow turn until the real head has been linked for a moment, then a
    /// slightly underdamped spring over to the listener's pose (mirror image).
    @MainActor
    func pose(_ flow: OnboardingFlow, at date: Date, dt: Double) -> HeadPose {
        let t = date.timeIntervalSince(flow.started)
        let following = [.linked, .screen, .keys].contains(flow.step) && flow.live
            && (flow.linkTime.map { date.timeIntervalSince($0) > 0.9 } ?? false)
        let stiffness = 28.0, damping = 2 * stiffness.squareRoot() * 0.72
        blendV += (((following ? 1 : 0) - blend) * stiffness - blendV * damping) * dt
        blend += blendV * dt
        let k = 1 - exp(-dt * 16)
        var dy = flow.real.yaw - real.yaw
        if dy > .pi { dy -= 2 * .pi } else if dy < -.pi { dy += 2 * .pi }
        real.yaw += dy * k
        real.pitch += (flow.real.pitch - real.pitch) * k
        real.roll += (flow.real.roll - real.roll) * k
        // Reduce Motion: she does not turn by herself.
        let idle = Motion.reduced ? HeadPose() : HeadPose.idle(t, amount: 0.9)
        return HeadPose.mix(idle, real, min(max(blend, 0), 1.08))
    }

    @MainActor
    func step(_ date: Date) -> Double {
        let dt = min(max(date.timeIntervalSince(last ?? date), 0), 0.05)
        last = date
        return dt
    }
}

/// One canvas at up to 30 fps: the lines and the wire, her portrait (or her eye, or her profile
/// and the screen), the cross.
private struct ArtStage: View {
    @ObservedObject var flow: OnboardingFlow
    @State private var anim = ArtAnimator()

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30)) { tl in
            Canvas { ctx, size in
                ctx.withCGContext { cg in
                    MainActor.assumeIsolated {
                        Art.draw(cg, size: size, flow: flow, anim: anim, date: tl.date)
                    }
                }
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// The drawing, in CoreGraphics (y down), in the art's own points.
@MainActor
enum Art {
    /// One dither cell, points.
    nonisolated static let cell: CGFloat = 2
    /// The shadow side's dots: a neutral, smoggy grey (the accent is kept for the listener).
    static let shade = Color(red: 0.36, green: 0.375, blue: 0.355)

    static func cg(_ c: Color) -> CGColor { NSColor(c).cgColor }

    static func rgb(_ c: Color) -> (UInt8, UInt8, UInt8) {
        let n = NSColor(c).usingColorSpace(.sRGB) ?? .white
        return (UInt8(n.redComponent * 255), UInt8(n.greenComponent * 255), UInt8(n.blueComponent * 255))
    }

    /// A dither frame at `origin` (points), one cell per `cell` points, no smoothing.
    static func put(_ ctx: CGContext, _ f: DitherFrame, at origin: CGPoint) {
        guard let img = f.image(paper: rgb(Ink.paper), shade: rgb(shade), accent: rgb(Ink.accent)) else { return }
        let rect = CGRect(x: origin.x, y: origin.y, width: CGFloat(f.cols) * cell, height: CGFloat(f.rows) * cell)
        ctx.saveGState()
        ctx.interpolationQuality = .none
        ctx.translateBy(x: rect.minX, y: rect.maxY)
        ctx.scaleBy(x: 1, y: -1)
        ctx.draw(img, in: CGRect(origin: .zero, size: rect.size))
        ctx.restoreGState()
    }

    static func draw(_ ctx: CGContext, size: CGSize, flow: OnboardingFlow, anim: ArtAnimator, date: Date) {
        let dt = anim.step(date)
        if flow.step == .centre {
            eyeShot(ctx, size: size, flow: flow, anim: anim, date: date, dt: dt)
            return
        }
        if flow.step == .keys {
            eyeBand(ctx, size: size, flow: flow, date: date)
            return
        }
        let w = Double(size.width), t = date.timeIntervalSince(flow.started)

        // Hum: a slow standing wave on the wires while nothing is linked yet.
        let unlinked = [.language, .intro, .waiting, .noAirPods, .denied].contains(flow.step)
        let hum = Motion.reduced || !unlinked ? 0 : 1.6 * sin(2 * .pi * 0.45 * t)
        // The link: one pulse down the wire (0.8 s), then the earbud takes the accent.
        var pulse: Double?
        var linked = false
        if let lt = flow.linkTime, [.linked, .screen].contains(flow.step) {
            let p = date.timeIntervalSince(lt) / 0.8
            if Motion.reduced || p >= 1 { linked = true } else if p > 0 { pulse = p }
        }

        // Two power lines across the top, quiet.
        ctx.setLineCap(.round)
        ctx.setStrokeColor(cg(Ink.hairline))
        ctx.setLineWidth(1)
        for (k, y0) in [14.0, 26.0].enumerated() {
            ctx.addPath(span(-6, y0, w + 6, y0 - 4, sag: 18 + Double(k) * 4, hum: hum * (k == 0 ? 1 : -0.7)))
        }
        ctx.strokePath()

        var look = PortraitLook()
        look.pose = anim.pose(flow, at: date, dt: dt)
        let profile = flow.step == .screen
        let portrait = profile ? SideLayout.portraitSize : 118
        let origin = profile ? SideLayout.portraitOrigin : CGPoint(x: 250 - Double(portrait), y: 20)
        var (frame, rawPod) = PortraitDither.head(look, cols: portrait, rows: portrait, size: portrait, profile: profile)
        // The earbud eases to where the views put it (a turn can hand it from one ear to the
        // other); a new shot (the profile) starts it in place.
        if anim.pod == nil || profile != anim.podProfile || dt == 0 {
            anim.pod = rawPod
        } else if let p = anim.pod {
            anim.pod = p + (rawPod - p) * (1 - exp(-dt * 14))
        }
        anim.podProfile = profile
        let pod = anim.pod ?? rawPod
        if linked { frame.tint(around: pod, radius: Double(portrait) * 0.035) }
        let podPoint = CGPoint(x: Double(origin.x) + pod.x * Double(cell), y: Double(origin.y) + pod.y * Double(cell))

        // The drop wire from the second line into her earbud, drawn under her.
        let a = CGPoint(x: Double(podPoint.x) + 30, y: 26 + 18 + 4)
        let pts = drop(from: a, to: podPoint, hum: hum * 1.5)
        put(ctx, frame, at: origin)
        ctx.addLines(between: pts)
        ctx.setStrokeColor(cg(linked ? Ink.graphite : Ink.hairline))
        ctx.setLineWidth(1)
        ctx.strokePath()
        if let p = pulse {
            let n = pts.count - 1
            let head = Int(Double(n) * min(1, p * 1.1)), tail = max(0, head - 6)
            if head > tail {
                ctx.addLines(between: Array(pts[tail...head]))
                ctx.setStrokeColor(cg(Ink.paper))
                ctx.setLineWidth(1.5)
                ctx.strokePath()
            }
        }

        if profile, let g = flow.geometry { sideView(ctx, geometry: g) }
    }

    static func span(_ x0: Double, _ y0: Double, _ x1: Double, _ y1: Double, sag: Double, hum: Double) -> CGPath {
        let p = CGMutablePath()
        for k in 0...24 {
            let u = Double(k) / 24
            let pt = CGPoint(x: x0 + (x1 - x0) * u, y: y0 + (y1 - y0) * u + sag * 4 * u * (1 - u) + hum * sin(.pi * u))
            if k == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
        }
        return p
    }

    static func drop(from a: CGPoint, to b: CGPoint, hum: Double) -> [CGPoint] {
        (0...30).map { k in
            let u = Double(k) / 30
            return CGPoint(x: Double(a.x) + (Double(b.x) - Double(a.x)) * u + (14 + hum) * sin(.pi * u),
                           y: Double(a.y) + (Double(b.y) - Double(a.y)) * (u * u * 0.35 + u * 0.65))
        }
    }

    // MARK: Step 3

    /// Her line of sight to the screen's centre at its elevation, a dashed eye level, the
    /// screen as a flat paper slab facing her at its height to her scale, values in mono.
    static func sideView(_ ctx: CGContext, geometry g: ScreenGeometry) {
        let eye = SideLayout.eye, c = SideLayout.centre(g)
        let a = g.elevation * .pi / 180
        let dir = (x: cos(a), y: -sin(a)), across = (x: sin(a), y: cos(a))
        let half = g.height * SideLayout.perMetre / 2
        let r = SideLayout.reach(g.distance)

        ctx.setLineWidth(1)
        ctx.setStrokeColor(cg(Ink.hairline))
        ctx.setLineDash(phase: 0, lengths: [2, 3])
        ctx.move(to: CGPoint(x: Double(eye.x) + 8, y: Double(eye.y)))
        ctx.addLine(to: CGPoint(x: Double(eye.x) + r + 24, y: Double(eye.y)))
        ctx.strokePath()
        ctx.setLineDash(phase: 0, lengths: [])
        ctx.setStrokeColor(cg(Ink.graphite))
        ctx.move(to: CGPoint(x: Double(eye.x) + dir.x * 8, y: Double(eye.y) + dir.y * 8))
        ctx.addLine(to: CGPoint(x: Double(c.x) - dir.x * 6, y: Double(c.y) - dir.y * 6))
        ctx.strokePath()

        let slab = CGMutablePath(), th = 2.0
        slab.addLines(between: [
            CGPoint(x: Double(c.x) + across.x * half - dir.x * th, y: Double(c.y) + across.y * half - dir.y * th),
            CGPoint(x: Double(c.x) + across.x * half + dir.x * th, y: Double(c.y) + across.y * half + dir.y * th),
            CGPoint(x: Double(c.x) - across.x * half + dir.x * th, y: Double(c.y) - across.y * half + dir.y * th),
            CGPoint(x: Double(c.x) - across.x * half - dir.x * th, y: Double(c.y) - across.y * half - dir.y * th),
        ])
        slab.closeSubpath()
        ctx.addPath(slab)
        ctx.setFillColor(cg(Ink.paper))
        ctx.fillPath()

        let mid = CGPoint(x: (Double(eye.x) + Double(c.x)) / 2 - across.x * 12, y: (Double(eye.y) + Double(c.y)) / 2 - across.y * 12)
        label(ctx, tr("%d cm", Int((g.distance * 100).rounded())) + "  " + SideLayout.degrees(g.elevation), at: mid, centred: true)
        label(ctx, String(format: "%.1f″", locale: L10n.locale, g.diagonalInches),
              at: CGPoint(x: Double(c.x) - across.x * half + 10, y: Double(c.y) - across.y * half + 2), centred: false)
    }

    static func label(_ ctx: CGContext, _ s: String, at p: CGPoint, centred: Bool) {
        let font = NSFont(name: "CoppiceMono-Regular", size: Ink.Size.caption) ?? .monospacedSystemFont(ofSize: Ink.Size.caption, weight: .regular)
        let a = NSAttributedString(string: s, attributes: [.font: font, .foregroundColor: NSColor(Ink.graphite)])
        let line = CTLineCreateWithAttributedString(a)
        let width = CTLineGetTypographicBounds(line, nil, nil, nil)
        ctx.saveGState()
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        ctx.textPosition = CGPoint(x: Double(p.x) - (centred ? width / 2 : 0), y: Double(p.y) + 4)
        CTLineDraw(line, ctx)
        ctx.restoreGState()
    }

    // MARK: Step 2

    /// Her eye across the art; on the window's (and the screen's) exact centre, below it, a
    /// small cross with a ring that closes as you hold still.
    static func eyeShot(_ ctx: CGContext, size: CGSize, flow: OnboardingFlow, anim: ArtAnimator, date: Date, dt: Double) {
        let t = date.timeIntervalSince(flow.started)
        let inStep = date.timeIntervalSince(flow.stepStart)
        // The window's centre, in the art's points (the art starts at the window's top-left).
        let cx = Double(OnboardingController.size.width) / 2, cy = Double(OnboardingController.size.height) / 2

        // Gaze: at you, then down to the cross; quick looks when you move.
        var target = inStep < 1.2 ? SIMD2<Double>(0, 0) : SIMD2<Double>(0, -1)
        if !flow.calibrated, flow.rate > 0.5 {
            target = SIMD2(max(-0.9, min(0.9, flow.real.yaw * 1.5)), max(-1, min(0.6, flow.real.pitch * 1.5)))
        }
        if dt == 0 {
            anim.gaze = target
            anim.gazeV = .zero
            anim.ring = flow.calibrated ? 1 : flow.stillness
        }
        let stiffness = 90.0, damping = 2 * stiffness.squareRoot() * 0.8
        anim.gazeV += ((target - anim.gaze) * stiffness - anim.gazeV * damping) * dt
        anim.gaze += anim.gazeV * dt
        anim.ring += ((flow.calibrated ? 1 : flow.stillness) - anim.ring) * (1 - exp(-dt * 14))

        // The push-in: the eye grows into its frame over the first 0.7 s.
        let push = Motion.reduced ? 1 : 1 - pow(1 - min(1, inStep / 0.7), 3)
        let cols = Int(size.width / cell), rows = Int((cy - 26) / Double(cell))
        var shot = EyeShot()
        shot.open = Motion.reduced ? 1 : Blink.openness(at: t, seed: 0.4)
        shot.gaze = anim.gaze
        shot.locked = flow.calibrated
        let imageSize = Int((190 + 24 * push).rounded())
        let frame = EyeDither.render(shot, cols: cols, rows: rows, size: imageSize, centre: SIMD2(Double(cols) / 2 + 6, Double(rows) * 0.5))
        put(ctx, frame, at: .zero)

        let dim = flow.signalLost && !flow.calibrated
        ctx.setLineCap(.butt)
        ctx.setStrokeColor(cg(Ink.paper.opacity(dim ? 0.4 : 0.9)))
        ctx.setLineWidth(1.5)
        ctx.move(to: CGPoint(x: cx - 5, y: cy)); ctx.addLine(to: CGPoint(x: cx + 5, y: cy))
        ctx.move(to: CGPoint(x: cx, y: cy - 5)); ctx.addLine(to: CGPoint(x: cx, y: cy + 5))
        ctx.strokePath()
        ctx.addArc(center: CGPoint(x: cx, y: cy), radius: 12, startAngle: 0, endAngle: 2 * .pi, clockwise: false)
        ctx.setStrokeColor(cg(Ink.hairline))
        ctx.setLineWidth(1)
        ctx.strokePath()
        if anim.ring > 0.005 {
            let end = -Double.pi / 2 + 2 * .pi * min(1, anim.ring)
            ctx.addArc(center: CGPoint(x: cx, y: cy), radius: 12, startAngle: -.pi / 2, endAngle: end, clockwise: false)
            ctx.setStrokeColor(cg(flow.calibrated ? Ink.accent : Ink.paper))
            ctx.strokePath()
        }
    }
}

extension Art {
    /// The last step's band: her eye across the art, the iris in the accent (it follows the
    /// picker), blinking now and then.
    static func eyeBand(_ ctx: CGContext, size: CGSize, flow: OnboardingFlow, date: Date) {
        let t = date.timeIntervalSince(flow.started)
        let cols = Int(size.width / cell), rows = max(8, Int(size.height / cell))
        var shot = EyeShot()
        shot.open = Motion.reduced ? 1 : Blink.openness(at: t, seed: 0.9)
        shot.gaze = SIMD2(0.15 * sin(t * 0.3), -0.1)
        shot.locked = true
        let frame = EyeDither.render(shot, cols: cols, rows: rows, size: 214,
                                     centre: SIMD2(Double(cols) / 2 + 6, Double(rows) * 0.5))
        put(ctx, frame, at: .zero)
    }
}

// MARK: - The side view's geometry

/// Where things sit in the last step's side view (art points): her profile on the left, her
/// eye, and the screen's centre at its distance (compressed: 0.3–2 m over 80–230 pt) and
/// elevation along the line of sight.
enum SideLayout {
    static let portraitSize = 100 // cells
    static let portraitOrigin = CGPoint(x: 4, y: 52)
    static var eye: CGPoint {
        CGPoint(x: Double(portraitOrigin.x) + Portraits.eyeProfile.x * Double(portraitSize) * Double(Art.cell),
                y: Double(portraitOrigin.y) + Portraits.eyeProfile.y * Double(portraitSize) * Double(Art.cell))
    }
    /// Her head is about half the portrait's height and 0.24 m tall.
    static var perMetre: Double { Double(portraitSize) * Double(Art.cell) * 0.5 / 0.24 }

    static func reach(_ distance: Double) -> Double {
        let u = (min(max(distance, 0.3), 2.0) - 0.3) / 1.7
        return 80 + 150 * pow(u, 0.7)
    }

    static func distance(reach r: Double) -> Double {
        0.3 + 1.7 * pow(max(0, min(1, (r - 80) / 150)), 1 / 0.7)
    }

    static func centre(_ g: ScreenGeometry) -> CGPoint {
        let r = reach(g.distance), a = g.elevation * .pi / 180
        return CGPoint(x: Double(eye.x) + r * cos(a), y: Double(eye.y) - r * sin(a))
    }

    static func degrees(_ e: Double) -> String {
        let d = Int(e.rounded())
        return (d < 0 ? "−" : d > 0 ? "+" : "") + "\(abs(d))°"
    }
}

/// The screen in the side view, draggable: across sets the distance, up and down the angle
/// from the eyes (the line of sight goes to where you let go).
private struct ScreenHandle: View {
    static let space = "onboarding.window"
    @ObservedObject var flow: OnboardingFlow
    @ObservedObject private var lang = Language.shared
    @State private var hovering = false

    var body: some View {
        if let g = flow.geometry {
            let c = SideLayout.centre(g)
            let h = g.height * SideLayout.perMetre + 20
            Color.clear
                .frame(width: 36, height: max(44, h))
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space)).onChanged { v in
                    let dx = Double(v.location.x - SideLayout.eye.x)
                    let dy = Double(SideLayout.eye.y - min(v.location.y, OnboardingView.artHeight - 4))
                    let r = max(1, (dx * dx + dy * dy).squareRoot())
                    flow.setDistance((SideLayout.distance(reach: r) * 100).rounded() / 100)
                    flow.setElevation((atan2(dy, max(dx, 1)) * 180 / .pi).rounded())
                })
                .onHover { inside in
                    if inside != hovering { inside ? NSCursor.openHand.push() : NSCursor.pop() }
                    hovering = inside
                }
                .accessibilityLabel(tr("Screen: drag to set the distance and angle"))
                // Placed last: `.position` fills the whole window, and a gesture attached after
                // it took every click in the window (the step's buttons could not be pressed).
                .position(c)
        }
    }
}
