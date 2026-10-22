import AppKit
import SwiftUI

/// The onboarding window, built like the popover so the hand-over into the panel feels like one
/// product: the art full-bleed on top (where the popover has its scene), then the panel's ground,
/// Coppice Mono at the panel's sizes, its greys and its buttons on the 4 pt grid. The art is
/// her portrait through an ordered dither (Dither.swift): paper on the lit side, red in the
/// shadow, violet only for the listener.
struct OnboardingView: View {
    @ObservedObject var flow: OnboardingFlow

    static let artHeight: CGFloat = 256

    var body: some View {
        ZStack(alignment: .topLeading) {
            Ink.panel
            VStack(alignment: .leading, spacing: 0) {
                ArtStage(flow: flow)
                    .frame(width: OnboardingController.size.width, height: Self.artHeight)
                    .background(Color.black)
                    .clipped()
                words
                    .padding(.horizontal, Grid.x4)
                    .padding(.top, Grid.x4)
                    .padding(.bottom, Grid.x4)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            if flow.step == .done, flow.geometry != nil {
                ScreenHandle(flow: flow)
            }
        }
        .coordinateSpace(name: ScreenHandle.space)
        .frame(width: OnboardingController.size.width, height: OnboardingController.size.height)
        .environment(\.colorScheme, .dark)
    }

    private var stepNumber: Int {
        switch flow.step {
        case .intro, .waiting, .linked, .noAirPods, .denied: return 1
        case .centre: return 2
        case .done: return 3
        }
    }

    // MARK: Words

    @ViewBuilder private var words: some View {
        switch flow.step {
        case .intro:
            Words(title: "соединяем голову со звуком", step: stepNumber,
                  lines: ["airpods знают, куда повёрнута голова: звук остаётся на месте, когда ты оборачиваешься."]) {
                PanelButton(title: "разрешить", primary: true) { flow.allow() }
                PanelButton(title: "без airpods") { flow.continueWithoutHead() }
            }
        case .waiting:
            Words(title: "соединяем голову со звуком", step: stepNumber,
                  lines: ["ждём сигнал от airpods.", "если macos спросит про доступ к движению, разреши."]) {
                PanelButton(title: "продолжить без airpods") { flow.continueWithoutHead() }
            }
        case .linked:
            Words(title: "наушники подключены", step: stepNumber,
                  lines: ["поверни голову: она повторяет за тобой."]) {
                PanelButton(title: "дальше", primary: true) { flow.toCentre() }
            }
        case .noAirPods:
            Words(title: "airpods не найдены", step: stepNumber,
                  lines: ["подключи их: продолжим сами, как только появятся.",
                          "или слушай без них: звук будет, просто не будет следить за головой."]) {
                PanelButton(title: "продолжить без airpods", primary: true) { flow.continueWithoutHead() }
            }
        case .denied:
            Words(title: "доступ к движению закрыт", step: stepNumber,
                  lines: ["открой его в системных настройках: конфиденциальность и безопасность."]) {
                PanelButton(title: "продолжить без airpods", primary: true) { flow.continueWithoutHead() }
                PanelButton(title: "открыть настройки") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        case .centre:
            Words(title: "посмотри в центр", step: stepNumber,
                  lines: [flow.calibrated ? "готово: вперёд теперь там, где экран."
                          : flow.signalLost ? "нет сигнала от airpods. проверь, что они в ушах."
                          : "на красный крест. замри на секунду."]) {
                PanelButton(title: "пропустить") { flow.skipCentre() }
                    .opacity(flow.calibrated ? 0 : 1)
            }
        case .done:
            Words(title: "последний штрих", step: stepNumber, lines: ["настрой экран"]) {
                PanelButton(title: "слушать", primary: true) { flow.finish(play: true) }
            } extra: {
                ScreenControls(flow: flow)
            }
        }
    }
}

/// Title (the popover's prompt size) with the step on the right, plain lines, optional
/// controls, the actions at the bottom.
private struct Words<Actions: View, Extra: View>: View {
    let title: String
    let step: Int
    let lines: [String]
    @ViewBuilder var actions: Actions
    @ViewBuilder var extra: Extra

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                Text(title)
                    .font(Ink.text(Ink.Size.prompt + 2))
                    .foregroundStyle(Ink.paper)
                Spacer(minLength: Grid.x2)
                Text("\(step) / 3")
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.ash)
            }
            VStack(alignment: .leading, spacing: Grid.x1) {
                ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            }
            .font(Ink.text(Ink.Size.body))
            .foregroundStyle(Ink.graphite)
            .padding(.top, Grid.x2)
            extra
                .padding(.top, Grid.x2)
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

/// The popover's button: primary filled with paper, the rest plain until hovered.
private struct PanelButton: View {
    let title: String
    var primary = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        let button = Button(action: action) {
            Text(title)
                .font(Ink.text(Ink.Size.title))
                .foregroundStyle(primary ? Ink.panel : hovering ? Ink.paper : Ink.graphite)
                .padding(.horizontal, Grid.x3)
                .frame(height: 28)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(primary ? (hovering ? Color(white: 1) : Ink.paper) : hovering ? Ink.raised : .clear))
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(primary ? .clear : Ink.hairline, lineWidth: 1))
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

/// The screen's distance and angle, each with plain steps and arrow keys.
private struct ScreenControls: View {
    @ObservedObject var flow: OnboardingFlow

    var body: some View {
        if let g = flow.geometry {
            VStack(alignment: .leading, spacing: Grid.x1) {
                row("до глаз", "\(Int((g.distance * 100).rounded())) см") {
                    Step(label: "−", key: .leftArrow, help: "Ближе к экрану") { flow.setDistance(g.distance - 0.05) }
                    Step(label: "+", key: .rightArrow, help: "Дальше от экрана") { flow.setDistance(g.distance + 0.05) }
                }
                row("угол", SideLayout.degrees(g.elevation)) {
                    Step(label: "↓", key: .downArrow, help: "Экран ниже") { flow.setElevation((g.elevation - 1).rounded()) }
                    Step(label: "↑", key: .upArrow, help: "Экран выше") { flow.setElevation((g.elevation + 1).rounded()) }
                    Text("или тяни экран на картинке").foregroundStyle(Ink.ash).padding(.leading, Grid.x2)
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
                    .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(hovering ? Ink.raised : .clear))
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
    var ring = 0.0
    var last: Date?

    /// Her pose: her own slow turn until the real head has been linked for a moment, then a
    /// slightly underdamped spring over to the listener's pose (mirror image).
    @MainActor
    func pose(_ flow: OnboardingFlow, at date: Date, dt: Double) -> HeadPose {
        let t = date.timeIntervalSince(flow.started)
        let following = [.linked, .done].contains(flow.step) && flow.live
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
    static let red = Color(red: 0xA3 / 255.0, green: 0x26 / 255.0, blue: 0x2E / 255.0)

    static func cg(_ c: Color) -> CGColor { NSColor(c).cgColor }

    static func rgb(_ c: Color) -> (UInt8, UInt8, UInt8) {
        let n = NSColor(c).usingColorSpace(.sRGB) ?? .white
        return (UInt8(n.redComponent * 255), UInt8(n.greenComponent * 255), UInt8(n.blueComponent * 255))
    }

    /// A dither frame at `origin` (points), one cell per `cell` points, no smoothing.
    static func put(_ ctx: CGContext, _ f: DitherFrame, at origin: CGPoint) {
        guard let img = f.image(paper: rgb(Ink.paper), red: rgb(red), violet: rgb(Ink.violet)) else { return }
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
        let w = Double(size.width), t = date.timeIntervalSince(flow.started)

        // Hum: a slow standing wave on the wires while nothing is linked yet.
        let unlinked = [.intro, .waiting, .noAirPods, .denied].contains(flow.step)
        let hum = Motion.reduced || !unlinked ? 0 : 1.6 * sin(2 * .pi * 0.45 * t)
        // The link: one pulse down the wire (0.8 s), then the earbud is violet.
        var pulse: Double?
        var linked = false
        if let lt = flow.linkTime, [.linked, .done].contains(flow.step) {
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
        look.linked = linked
        let profile = flow.step == .done
        let portrait = profile ? SideLayout.portraitSize : 118
        let origin = profile ? SideLayout.portraitOrigin : CGPoint(x: 250 - Double(portrait), y: 20)
        let (frame, pod) = PortraitDither.head(look, cols: portrait, rows: portrait, size: portrait, profile: profile)
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
        label(ctx, "\(Int((g.distance * 100).rounded())) см  \(SideLayout.degrees(g.elevation))", at: mid, centred: true)
        label(ctx, String(format: "%.1f″", g.diagonalInches),
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
    /// small red cross with a ring that closes as you hold still.
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
        ctx.setStrokeColor(cg(red.opacity(dim ? 0.45 : 1)))
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
            ctx.setStrokeColor(cg(flow.calibrated ? Ink.violet : Ink.paper))
            ctx.strokePath()
        }
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
    @State private var hovering = false

    var body: some View {
        if let g = flow.geometry {
            let c = SideLayout.centre(g)
            let h = g.height * SideLayout.perMetre + 20
            Color.clear
                .frame(width: 36, height: max(44, h))
                .contentShape(Rectangle())
                .position(c)
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
                .accessibilityLabel("Экран: тяни, чтобы задать расстояние и угол")
        }
    }
}
