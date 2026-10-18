import SwiftUI

/// Minimal SVG path data parser (M L H V C S Q T Z, absolute and relative) → SwiftUI Path.
enum SVGPath {
    static func parse(_ d: String) -> Path {
        var path = Path()
        var tokens: [Substring] = []
        var i = d.startIndex
        while i < d.endIndex {
            let c = d[i]
            if c.isLetter {
                tokens.append(d[i...i])
                i = d.index(after: i)
            } else if c == "-" || c == "." || c.isNumber {
                var j = d.index(after: i)
                var seenDot = c == "."
                while j < d.endIndex {
                    let k = d[j]
                    if k.isNumber { j = d.index(after: j); continue }
                    if k == "." && !seenDot { seenDot = true; j = d.index(after: j); continue }
                    if (k == "e" || k == "E") { j = d.index(after: j); if j < d.endIndex, d[j] == "-" { j = d.index(after: j) }; continue }
                    break
                }
                tokens.append(d[i..<j])
                i = j
            } else {
                i = d.index(after: i)
            }
        }
        var cur = CGPoint.zero, start = CGPoint.zero, lastCtrl: CGPoint?
        var cmd: Character = "M"
        var k = 0
        func num() -> CGFloat { defer { k += 1 }; return k < tokens.count ? CGFloat(Double(tokens[k]) ?? 0) : 0 }
        func pt(_ rel: Bool) -> CGPoint { let x = num(), y = num(); return rel ? CGPoint(x: cur.x + x, y: cur.y + y) : CGPoint(x: x, y: y) }
        while k < tokens.count {
            if let c = tokens[k].first, c.isLetter { cmd = c; k += 1 }
            let rel = cmd.isLowercase
            switch cmd.uppercased().first! {
            case "M":
                cur = pt(rel); start = cur; path.move(to: cur)
                cmd = rel ? "l" : "L"
            case "L": cur = pt(rel); path.addLine(to: cur)
            case "H": let x = num(); cur.x = rel ? cur.x + x : x; path.addLine(to: cur)
            case "V": let y = num(); cur.y = rel ? cur.y + y : y; path.addLine(to: cur)
            case "C":
                let c1 = pt(rel), c2 = pt(rel), p = pt(rel)
                path.addCurve(to: p, control1: c1, control2: c2); lastCtrl = c2; cur = p; continue
            case "S":
                let c1 = lastCtrl.map { CGPoint(x: 2 * cur.x - $0.x, y: 2 * cur.y - $0.y) } ?? cur
                let c2 = pt(rel), p = pt(rel)
                path.addCurve(to: p, control1: c1, control2: c2); lastCtrl = c2; cur = p; continue
            case "Q":
                let c = pt(rel), p = pt(rel)
                path.addQuadCurve(to: p, control: c); lastCtrl = c; cur = p; continue
            case "T":
                let c = lastCtrl.map { CGPoint(x: 2 * cur.x - $0.x, y: 2 * cur.y - $0.y) } ?? cur
                let p = pt(rel)
                path.addQuadCurve(to: p, control: c); lastCtrl = c; cur = p; continue
            case "Z":
                path.closeSubpath(); cur = start
            default:
                k += 1
            }
            lastCtrl = nil
        }
        return path
    }
}

/// The icons, drawn as SVG paths on a 24 x 24 view box with a fine engraved stroke. Each takes
/// a time `t` (seconds) and a level so it can animate with its sound.
enum Icon: Hashable {
    case water, wind, birds, flight, volume
    case play, pause, recenter, headphones, quit
    case tree, bird, river

    struct Stroke {
        var d: String
        var opacity: Double = 1
        var dash: [CGFloat] = []
        var dashPhase: CGFloat = 0
    }

    private static func f(_ v: Double) -> String { String(format: "%.2f", v) }

    /// A sine run as a smooth path of cubic pieces.
    private static func wave(y: Double, amp: Double, x0: Double, x1: Double, period: Double, phase: Double) -> String {
        let steps = 10
        var d = ""
        let dx = (x1 - x0) / Double(steps)
        func yAt(_ x: Double) -> Double { y + amp * sin((x - x0) / period * 2 * .pi - phase) }
        func slope(_ x: Double) -> Double { amp * cos((x - x0) / period * 2 * .pi - phase) * 2 * .pi / period }
        d += "M\(f(x0)) \(f(yAt(x0)))"
        for i in 0..<steps {
            let a = x0 + Double(i) * dx, b = a + dx
            d += " C\(f(a + dx / 3)) \(f(yAt(a) + slope(a) * dx / 3)) \(f(b - dx / 3)) \(f(yAt(b) - slope(b) * dx / 3)) \(f(b)) \(f(yAt(b)))"
        }
        return d
    }

    func strokes(t: Double, level: Double) -> [Stroke] {
        let f = Icon.f
        switch self {
        case .water, .river:
            let ph = t * 2.2
            return [
                Stroke(d: Icon.wave(y: 8, amp: 1.6, x0: 2, x1: 22, period: 8, phase: ph)),
                Stroke(d: Icon.wave(y: 13, amp: 1.6, x0: 2, x1: 22, period: 8, phase: ph + 1.8), opacity: 0.8),
                Stroke(d: Icon.wave(y: 18, amp: 1.2, x0: 5, x1: 19, period: 7, phase: ph + 3.4), opacity: 0.55),
            ]
        case .wind:
            // Streamlines that end in the curl of an old map's wind.
            let flow = CGFloat(t * 9)
            return [
                Stroke(d: "M2 8 H14 C18 8 19 4 16 3.5 C14 3.2 13.5 5.5 15 6", dash: [9, 3], dashPhase: -flow),
                Stroke(d: "M2 13 H18 C21.5 13 22 17.5 19 18 C17 18.3 16.4 16 18 15.5", opacity: 0.85, dash: [11, 3], dashPhase: -flow * 1.2),
                Stroke(d: "M5 18 H11", opacity: 0.55, dash: [4, 2], dashPhase: -flow * 0.8),
            ]
        case .birds, .bird:
            let flap = sin(t * 9) * 2.2 * min(1, level * 3 + 0.25)
            let flap2 = sin(t * 9 + 1.7) * 1.6 * min(1, level * 3 + 0.25)
            return [
                Stroke(d: "M2 \(f(9 - flap)) Q6.5 \(f(6.5 - flap)) 10 11.5 Q13.5 \(f(6.5 - flap)) 18 \(f(9 - flap))"),
                Stroke(d: "M13 \(f(16 - flap2)) Q15.5 \(f(14.5 - flap2)) 17.5 17.5 Q19.5 \(f(14.5 - flap2)) 22 \(f(16 - flap2))", opacity: 0.6),
            ]
        case .flight:
            let a = t * 1.6
            let bx = 12 + 9 * cos(a), by = 12 + 5 * sin(a)
            return [
                Stroke(d: "M3 12 C3 8.7 7 7 12 7 C17 7 21 8.7 21 12 C21 15.3 17 17 12 17 C7 17 3 15.3 3 12 Z", opacity: 0.45, dash: [1.2, 2.2]),
                Stroke(d: "M12 10.6 C12.8 10.6 13.4 11.2 13.4 12 C13.4 12.8 12.8 13.4 12 13.4 C11.2 13.4 10.6 12.8 10.6 12 C10.6 11.2 11.2 10.6 12 10.6 Z", opacity: 0.7),
                Stroke(d: "M\(f(bx - 2.2)) \(f(by - 1)) Q\(f(bx - 1)) \(f(by - 1.8)) \(f(bx)) \(f(by)) Q\(f(bx + 1)) \(f(by - 1.8)) \(f(bx + 2.2)) \(f(by - 1))"),
            ]
        case .volume:
            let p = 0.25 + 0.75 * min(1, level * 1.5)
            let pulse = 0.5 + 0.5 * sin(t * 6)
            return [
                Stroke(d: "M3 9.5 H7 L12 5.5 V18.5 L7 14.5 H3 Z"),
                Stroke(d: "M15 9.5 Q16.8 12 15 14.5", opacity: p),
                Stroke(d: "M17.5 7.5 Q20.6 12 17.5 16.5", opacity: p * (0.5 + 0.5 * pulse)),
                Stroke(d: "M20 5.5 Q24.2 12 20 18.5", opacity: p * pulse * 0.8),
            ]
        case .play:
            return [Stroke(d: "M8 5.5 L18.5 12 L8 18.5 Z")]
        case .pause:
            return [Stroke(d: "M9 6 V18 M15 6 V18")]
        case .recenter:
            let r = 6.5 + 0.6 * sin(t * 3)
            return [
                Stroke(d: "M\(f(12 - r)) 12 C\(f(12 - r)) \(f(12 - r * 0.55)) \(f(12 - r * 0.55)) \(f(12 - r)) 12 \(f(12 - r)) C\(f(12 + r * 0.55)) \(f(12 - r)) \(f(12 + r)) \(f(12 - r * 0.55)) \(f(12 + r)) 12 C\(f(12 + r)) \(f(12 + r * 0.55)) \(f(12 + r * 0.55)) \(f(12 + r)) 12 \(f(12 + r)) C\(f(12 - r * 0.55)) \(f(12 + r)) \(f(12 - r)) \(f(12 + r * 0.55)) \(f(12 - r)) 12 Z", opacity: 0.8),
                Stroke(d: "M12 2.5 V6 M12 18 V21.5 M2.5 12 H6 M18 12 H21.5"),
                Stroke(d: "M12 11 V13 M11 12 H13"),
            ]
        case .headphones:
            return [
                Stroke(d: "M5 15 V12 C5 3.5 19 3.5 19 12 V15"),
                Stroke(d: "M3.5 14 H7 V20 H3.5 Z M17 14 H20.5 V20 H17 Z"),
            ]
        case .quit:
            return [Stroke(d: "M7 7 L17 17 M17 7 L7 17")]
        case .tree:
            let sway = sin(t * 1.5) * 0.6
            return [
                Stroke(d: "M\(f(12 + sway)) 3 C7 5 5 9.5 7.5 12 C5 14 7 17.5 \(f(12 + sway * 0.5)) 16.5 C17 17.5 19 14 16.5 12 C19 9.5 17 5 \(f(12 + sway)) 3 Z"),
                Stroke(d: "M12 16.5 V21.5 M9.5 21.5 H14.5"),
            ]
        }
    }
}

extension Icon {
    /// Length of one animation loop, seconds (nil: the icon never moves).
    var period: Double? {
        switch self {
        case .water, .river: return 2 * .pi / 2.2
        case .birds, .bird: return 2 * .pi / 9
        case .flight: return 2 * .pi / 1.6
        case .volume: return 2 * .pi / 6
        case .recenter: return 2 * .pi / 3
        case .tree: return 2 * .pi / 1.5
        case .wind: return 4.0 / 3.0
        case .play, .pause, .headphones, .quit: return nil
        }
    }
}

/// Draws an icon. Its animation is baked once into path keyframes and run by Core Animation,
/// so a moving icon costs the app next to nothing; `animating` only starts or stops it.
struct IconView: View {
    let icon: Icon
    var color: Color = Ink.paper
    var size: CGFloat = 18
    var level: Double = 0
    var animating = false
    var struck = false
    var lineWidth: CGFloat = 1.1

    /// Snapshot mode renders through ImageRenderer, which cannot see AppKit layers.
    static var still = false

    var body: some View {
        Group {
            if Self.still {
                Canvas { ctx, sz in
                    let k = sz.width / 24
                    ctx.scaleBy(x: k, y: k)
                    for s in icon.strokes(t: 0, level: max(level, 0.5)) {
                        ctx.stroke(SVGPath.parse(s.d), with: .color(color.opacity(s.opacity)),
                                   style: StrokeStyle(lineWidth: lineWidth / k, lineCap: .round, lineJoin: .round, dash: s.dash))
                    }
                    if struck {
                        ctx.stroke(SVGPath.parse("M3 21 L21 3"), with: .color(color), style: StrokeStyle(lineWidth: lineWidth / k, lineCap: .round))
                    }
                }
            } else {
                IconLayer(icon: icon, color: NSColor(color), animating: animating, struck: struck, lineWidth: lineWidth)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

private struct IconLayer: NSViewRepresentable {
    let icon: Icon
    let color: NSColor
    let animating: Bool
    let struck: Bool
    let lineWidth: CGFloat

    func makeNSView(context: Context) -> IconLayerView { IconLayerView() }

    func updateNSView(_ v: IconLayerView, context: Context) {
        v.configure(icon: icon, color: color, animating: animating, struck: struck, lineWidth: lineWidth)
    }
}

final class IconLayerView: NSView {
    private var icon: Icon?
    private var builtSize: CGSize = .zero
    private var strokes: [CAShapeLayer] = []
    private let strike = CAShapeLayer()
    private var running = false
    private var color = NSColor.white
    private var lineWidth: CGFloat = 1.1

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.isGeometryFlipped = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        if bounds.size != builtSize, let icon { build(icon) }
    }

    func configure(icon: Icon, color: NSColor, animating: Bool, struck: Bool, lineWidth: CGFloat) {
        self.color = color
        self.lineWidth = lineWidth
        if icon != self.icon || bounds.size != builtSize {
            self.icon = icon
            build(icon)
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for l in strokes { l.strokeColor = color.cgColor }
        strike.strokeColor = color.cgColor
        strike.isHidden = !struck
        CATransaction.commit()
        setRunning(animating)
    }

    private func path(_ d: String, scale k: CGFloat) -> CGPath {
        var t = CGAffineTransform(scaleX: k, y: k)
        return SVGPath.parse(d).cgPath.copy(using: &t) ?? CGMutablePath()
    }

    private func build(_ icon: Icon) {
        builtSize = bounds.size
        strokes.forEach { $0.removeFromSuperlayer() }
        strokes = []
        guard bounds.width > 0 else { return }
        let k = bounds.width / 24
        for s in icon.strokes(t: 0, level: 0.6) {
            let l = CAShapeLayer()
            l.fillColor = nil
            l.lineWidth = lineWidth
            l.lineCap = .round
            l.lineJoin = .round
            l.path = path(s.d, scale: k)
            l.opacity = Float(s.opacity)
            if !s.dash.isEmpty { l.lineDashPattern = s.dash.map { NSNumber(value: Double($0 * k)) } }
            layer?.addSublayer(l)
            strokes.append(l)
        }
        strike.path = path("M3 21 L21 3", scale: k)
        strike.lineWidth = lineWidth
        strike.lineCap = .round
        strike.fillColor = nil
        layer?.addSublayer(strike)
        running = false
    }

    private func setRunning(_ on: Bool) {
        guard let icon, on != running else { return }
        running = on
        guard on, let period = icon.period else {
            strokes.forEach { $0.removeAllAnimations() }
            return
        }
        let k = bounds.width / 24
        let frames = 36
        let samples = (0...frames).map { icon.strokes(t: period * Double($0) / Double(frames), level: 0.6) }
        for (i, l) in strokes.enumerated() {
            let first = samples[0][i]
            if !first.dash.isEmpty {
                // Dashes flow along the stroke.
                let total = first.dash.reduce(0, +) * k
                let a = CABasicAnimation(keyPath: "lineDashPhase")
                a.fromValue = 0
                a.toValue = -total
                a.duration = Double(total / k) / 9
                a.repeatCount = .infinity
                l.add(a, forKey: "flow")
                continue
            }
            let paths = samples.map { path($0[i].d, scale: k) }
            if paths.dropFirst().contains(where: { $0 != paths[0] }) {
                let a = CAKeyframeAnimation(keyPath: "path")
                a.values = paths
                a.duration = period
                a.repeatCount = .infinity
                a.calculationMode = .linear
                l.add(a, forKey: "shape")
            }
            let opacities = samples.map { Float($0[i].opacity) }
            if opacities.contains(where: { $0 != opacities[0] }) {
                let a = CAKeyframeAnimation(keyPath: "opacity")
                a.values = opacities
                a.duration = period
                a.repeatCount = .infinity
                l.add(a, forKey: "fade")
            }
        }
    }
}

/// The palette: silver-white cut into black, and two greys between.
enum Ink {
    static let black = Color(white: 0)
    static let paper = Color(white: 0.94)
    static let graphite = Color(white: 0.55)
    static let ash = Color(white: 0.2)

    /// Cochin: cut from 18th-century French copperplate lettering.
    static func text(_ size: CGFloat = 12) -> Font { .custom("Cochin", size: size) }
    static func italic(_ size: CGFloat = 12) -> Font { .custom("Cochin-Italic", size: size) }
}
