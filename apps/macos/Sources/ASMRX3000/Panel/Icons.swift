import CAsmr
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
    case play, pause, recenter, headphones, head, quit
    case power, chevron, back, more
    case tree, bird, river
    // Child parameters and the newer groups.
    case flow, depth, character, speed, gust, rain, dot
    case sun, moon, thermometer, droplet, leaf, flame, cricket, bolt, bubbles, width, stone, tone, gain
    // Clip-scene groups.
    case whale, dolphin, comet, beam, orbit, plasma

    /// The pictogram for a control: its group for a macro, its key for a child. `hour` is the
    /// scene's time of day, so the environment shows a sun or a moon.
    static func forControl(_ c: Control, hour: Float? = nil) -> Icon {
        let daylight = hour.map { $0 >= 6 && $0 < 19 } ?? true
        if let icon = ControlText.clipIcon(c) { return icon }
        if c.group >= AudioEngine.sceneGroup {
            switch c.key {
            case "scene.motion": return .speed
            case "scene.events": return .dot
            case "scene.bed": return .tone
            default: return .bubbles
            }
        }
        if c.isMacro || c.isLevel {
            switch Int(c.group) {
            case ASMR_GROUP_WATER: return .water
            case ASMR_GROUP_WIND: return .wind
            case ASMR_GROUP_FOLIAGE: return .tree
            case ASMR_GROUP_RAIN: return .rain
            case ASMR_GROUP_BIRDS: return .birds
            case ASMR_GROUP_ENVIRONMENT: return daylight ? .sun : .moon
            case ASMR_GROUP_INSECTS: return .cricket
            case ASMR_GROUP_FIRE: return .flame
            case ASMR_GROUP_FLIGHT: return .flight
            case ASMR_GROUP_MIX: return .tone
            default: return .volume
            }
        }
        let leaf = c.key.split(separator: ".").last.map(String.init) ?? c.key
        if c.group == UInt32(ASMR_GROUP_MIX) {
            // Mix intent: focus per group shows that group's pictogram.
            switch leaf {
            case "water": return .water
            case "wind": return .wind
            case "birds", "soloists": return .birds
            case "rain": return .rain
            case "insects": return .cricket
            case "fire": return .flame
            case "density": return .dot
            default: return .gain
            }
        }
        switch leaf {
        case "flow", "current": return .flow
        case "speed" where c.group == UInt32(ASMR_GROUP_WATER): return .flow
        case "depth", "level": return .depth
        case "character", "preset", "texture", "regime": return .character
        case "speed", "strength", "exposure": return .speed
        case "gust", "gustiness", "turbulence": return .gust
        case "flight", "orbit": return .flight
        case "rate", "intensity": return .rain
        case "time_of_day": return daylight ? .sun : .moon
        case "temperature", "warmth": return .thermometer
        case "humidity", "moisture", "resin": return .droplet
        case "season", "density", "dryness", "flutter": return .leaf
        case "heat": return .flame
        case "storm": return .bolt
        case "aeration": return .bubbles
        case "width": return .width
        case "rock_size", "surface": return .stone
        case "whistle": return .tone
        case "gain", "volume": return .gain
        case "drying": return .sun
        case "creak": return .tree
        case "activity":
            switch Int(c.group) {
            case ASMR_GROUP_INSECTS: return .cricket
            case ASMR_GROUP_BIRDS: return .birds
            default: return .dot
            }
        default: return .dot
        }
    }

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

    /// A circle as four cubic arcs.
    static func circle(_ cx: Double, _ cy: Double, _ r: Double) -> String {
        let k = 0.5523 * r
        return "M\(f(cx - r)) \(f(cy)) C\(f(cx - r)) \(f(cy - k)) \(f(cx - k)) \(f(cy - r)) \(f(cx)) \(f(cy - r)) "
            + "C\(f(cx + k)) \(f(cy - r)) \(f(cx + r)) \(f(cy - k)) \(f(cx + r)) \(f(cy)) "
            + "C\(f(cx + r)) \(f(cy + k)) \(f(cx + k)) \(f(cy + r)) \(f(cx)) \(f(cy + r)) "
            + "C\(f(cx - k)) \(f(cy + r)) \(f(cx - r)) \(f(cy + k)) \(f(cx - r)) \(f(cy)) Z"
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
        case .head:
            // A head seen from the front with an AirPod in each ear, and the arc of a turn above.
            let turn = sin(t * 2) * 0.8
            return [
                Stroke(d: "M12 6.5 C15 6.5 16.6 8.8 16.6 11.6 C16.6 14.8 14.6 17 12 17 C9.4 17 7.4 14.8 7.4 11.6 C7.4 8.8 9 6.5 12 6.5 Z"),
                Stroke(d: "M6.6 10.6 C5.4 10.6 5 11.4 5 12 C5 12.7 5.5 13.2 6.2 13.2 V16.4 M17.4 10.6 C18.6 10.6 19 11.4 19 12 C19 12.7 18.5 13.2 17.8 13.2 V16.4"),
                Stroke(d: "M\(f(7.5 + turn)) 3.6 Q12 1.4 \(f(16.5 + turn)) 3.6 M\(f(14.6 + turn)) 2.4 L\(f(16.5 + turn)) 3.6 L\(f(15 + turn)) 5.2", opacity: 0.75),
                Stroke(d: "M8.5 21.5 C8.5 19.6 10 18.6 12 18.6 C14 18.6 15.5 19.6 15.5 21.5", opacity: 0.6),
            ]
        case .quit:
            return [Stroke(d: "M7 7 L17 17 M17 7 L7 17")]
        case .power:
            return [Stroke(d: "M8 6.8 C5.6 8.2 4.5 10.4 4.5 12.8 C4.5 17 7.9 20 12 20 C16.1 20 19.5 17 19.5 12.8 C19.5 10.4 18.4 8.2 16 6.8"),
                    Stroke(d: "M12 3.5 V11.5")]
        case .chevron:
            return [Stroke(d: "M9.5 6 L15.5 12 L9.5 18")]
        case .back:
            return [Stroke(d: "M14.5 6 L8.5 12 L14.5 18")]
        case .more:
            return [Stroke(d: Icon.circle(6, 12, 0.9)), Stroke(d: Icon.circle(12, 12, 0.9)), Stroke(d: Icon.circle(18, 12, 0.9))]
        case .flow:
            return [Stroke(d: "M3 13 C6.5 10 9.5 16 13 13 C15 11.3 17 11.6 20 13 M17 10 L20.5 13 L17 16")]
        case .depth:
            return [
                Stroke(d: "M3 7 C5.5 5.5 8 8.5 12 7 C15 5.8 17.5 8.2 21 7"),
                Stroke(d: "M12 10 V20 M9.5 17.5 L12 20 L14.5 17.5", opacity: 0.85),
            ]
        case .character:
            return [
                Stroke(d: "M4 18 Q12 4 20 18"),
                Stroke(d: "M8 18 Q12 10 16 18", opacity: 0.75),
                Stroke(d: "M11 18 Q12 15.5 13 18", opacity: 0.55),
            ]
        case .speed:
            return [Stroke(d: "M5 3 V21 M5 5 L19 7.5 L19 11.5 L5 13 M10 5.9 V12.4 M14.5 6.7 V12")]
        case .gust:
            return [
                Stroke(d: "M3 9 H13 C17 9 17 4 14 4.5"),
                Stroke(d: "M3 14 H18 C21 14 21 19 18 18.5", opacity: 0.8),
                Stroke(d: "M3 19 H9", opacity: 0.55),
            ]
        case .rain:
            let y = (t / 0.8).truncatingRemainder(dividingBy: 1) * 3
            return [
                Stroke(d: "M8 \(f(3 + y)) L6.5 \(f(7 + y)) M13 \(f(3 + y)) L11.5 \(f(7 + y)) M18 \(f(3 + y)) L16.5 \(f(7 + y))"),
                Stroke(d: "M10.5 \(f(11 + y)) L9 \(f(15 + y)) M15.5 \(f(11 + y)) L14 \(f(15 + y))", opacity: 0.7),
            ]
        case .dot:
            return [Stroke(d: Icon.circle(12, 12, 3))]
        case .sun:
            // Rays turn slowly around the disc.
            let a0 = t * 2 * .pi / 12
            var rays = ""
            for i in 0..<8 {
                let a = a0 + Double(i) * .pi / 4
                rays += "M\(f(12 + 6.3 * cos(a))) \(f(12 + 6.3 * sin(a))) L\(f(12 + 8.8 * cos(a))) \(f(12 + 8.8 * sin(a))) "
            }
            return [Stroke(d: Icon.circle(12, 12, 4)), Stroke(d: rays, opacity: 0.8)]
        case .moon:
            let tw = 0.35 + 0.65 * (0.5 + 0.5 * sin(t * 2.1))
            return [
                Stroke(d: "M15 4.5 C10 5 7 8.5 7 12.5 C7 16.5 10.5 19.5 14.5 19.5 C16.5 19.5 18 18.8 19.5 17.8 C14.5 17.5 11.5 14 11.5 10 C11.5 7.8 13 5.8 15 4.5 Z"),
                Stroke(d: "M19 6 V8 M18 7 H20", opacity: tw),
            ]
        case .thermometer:
            return [
                Stroke(d: "M10 4 C10 2.9 10.9 2 12 2 C13.1 2 14 2.9 14 4 V14 C15.2 14.7 16 16 16 17.5 C16 19.7 14.2 21.5 12 21.5 C9.8 21.5 8 19.7 8 17.5 C8 16 8.8 14.7 10 14 Z"),
                Stroke(d: "M12 8 V17.5", opacity: 0.8),
            ]
        case .droplet:
            return [Stroke(d: "M12 3 C9 8 6.5 11 6.5 14.5 C6.5 17.8 9 20.5 12 20.5 C15 20.5 17.5 17.8 17.5 14.5 C17.5 11 15 8 12 3 Z")]
        case .leaf:
            return [Stroke(d: "M5 19 C5 10 10 5 19 5 C19 14 14 19 5 19 Z"), Stroke(d: "M5 19 L13.5 10.5", opacity: 0.7)]
        case .flame:
            // The tip licks from side to side; the inner tongue breathes.
            let f1 = sin(t * 9) * 1.3, f2 = sin(t * 13 + 1) * 1.2
            return [
                Stroke(d: "M\(f(12 + f1)) 3 C\(f(13.5 + f1 * 0.5)) 7 17 9.5 17 14 C17 17.6 14.8 20.5 12 20.5 C9.2 20.5 7 17.6 7 14 C7 10.5 10 9 10.5 6 C11.3 7.6 \(f(12 + f1 * 0.5)) 8.5 \(f(12 + f1)) 3 Z"),
                Stroke(d: "M12 20.5 C10.5 20.5 9.5 19 9.5 17.3 C9.5 15.2 11.3 14.2 12 \(f(12 + f2)) C12.7 14.2 14.5 15.2 14.5 17.3 C14.5 19 13.5 20.5 12 20.5 Z", opacity: 0.7),
            ]
        case .cricket:
            let a = sin(t * 8) * 1.4
            return [
                Stroke(d: "M5 15 C5 12.6 8.5 11.2 12.5 11.2 C16 11.2 18.5 12.6 18.5 14.2 C18.5 15.7 16.2 16.6 12.5 16.6 C8.5 16.6 5 16.4 5 15 Z"),
                Stroke(d: "M9 16.4 L7.5 20 M13.5 16.6 L15 20 M11 11.4 L13.5 6.5 L16 9.5", opacity: 0.8),
                Stroke(d: "M18.5 13.5 C20 10 20.8 8 \(f(22 + a * 0.3)) \(f(5 + a)) M18 12.8 C18.6 9 18.8 7 \(f(19.5 - a * 0.3)) \(f(4 - a))", opacity: 0.7),
            ]
        case .bolt:
            return [Stroke(d: "M13 2.5 L6.5 13 H11.5 L10 21.5 L17.5 10 H12.5 Z")]
        case .bubbles:
            let r = 0.4 * sin(t * 4)
            return [
                Stroke(d: Icon.circle(9, 15, 3.5 + r)),
                Stroke(d: Icon.circle(15.5, 9.5, 2.5 - r * 0.5), opacity: 0.8),
                Stroke(d: Icon.circle(17, 16.5, 1.4), opacity: 0.6),
            ]
        case .width:
            return [Stroke(d: "M4 6 V18 M20 6 V18"), Stroke(d: "M4.5 12 H19.5 M7 9.5 L4.5 12 L7 14.5 M17 9.5 L19.5 12 L17 14.5", opacity: 0.8)]
        case .stone:
            return [Stroke(d: "M4.5 16.5 L7 10.5 L12 8 L17 9.5 L19.5 15 L15.5 18.5 L8 19 Z"), Stroke(d: "M7 10.5 L11 13 L17 9.5 M11 13 L12 18.8", opacity: 0.55)]
        case .tone:
            return [Stroke(d: Icon.wave(y: 12, amp: 3.5, x0: 3, x1: 21, period: 6, phase: t * 3))]
        case .gain:
            return [Stroke(d: "M4 9.5 H8 L13 5.5 V18.5 L8 14.5 H4 Z"), Stroke(d: "M16 12 H20", opacity: 0.8)]
        case .whale:
            return [
                Stroke(d: "M3 14.5 C4 10 9 8.5 14 10 C16.5 10.8 18 12.2 19.5 14 C18.5 11.5 19.5 8.5 22 7.5 M19.5 14 C20 16 21 17 22.5 17.5"),
                Stroke(d: "M3 14.5 C6 18 13 18 17.5 15.2", opacity: 0.8),
                Stroke(d: "M8.5 8.3 V4.5 M6.8 6 L8.5 4.5 L10.2 6", opacity: 0.6),
            ]
        case .dolphin:
            return [
                Stroke(d: "M3 15.5 C5 9.5 11 7 16.5 9 L21.5 7 C20.5 9.2 20 10.2 19.2 11.2 C17 15.5 9 18.5 3 15.5 Z"),
                Stroke(d: "M10.5 7.6 L12 4 L14.5 8.2", opacity: 0.8),
                Stroke(d: "M12 15.4 L10.5 18.5 L14 16.6", opacity: 0.6),
            ]
        case .comet:
            return [
                Stroke(d: Icon.circle(17, 7, 2.6)),
                Stroke(d: "M14.7 8.6 L3.5 19.5 M15.6 9.8 L8.5 20 M13 6.4 L3 13", opacity: 0.7),
            ]
        case .beam:
            return [
                Stroke(d: "M3 12 H21"),
                Stroke(d: "M6 8 L9 12 L6 16 M12 8 L15 12 L12 16", opacity: 0.75),
                Stroke(d: "M18.5 9.5 V14.5", opacity: 0.5),
            ]
        case .orbit:
            return [
                Stroke(d: "M3.5 12 C3.5 7 20.5 7 20.5 12 C20.5 17 3.5 17 3.5 12 Z", opacity: 0.8),
                Stroke(d: Icon.circle(12, 12, 2.2)),
                Stroke(d: Icon.circle(19.6, 14.4, 1.2), opacity: 0.7),
            ]
        case .plasma:
            return [
                Stroke(d: "M3 5 C9 5 10.5 11 12.5 13.5 C14.5 16 16.5 18 21 19"),
                Stroke(d: "M4 17 H7 M9 19.5 H11.5 M15 4.5 H18 M17 8 H20.5", opacity: 0.55),
            ]
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
        case .head: return .pi
        case .tree: return 2 * .pi / 1.5
        case .wind: return 4.0 / 3.0
        case .rain: return 0.8
        case .sun: return 12
        case .moon: return 2 * .pi / 2.1
        case .flame: return 2 * .pi
        case .cricket: return 2 * .pi / 8
        case .bubbles: return 2 * .pi / 4
        case .tone: return 2 * .pi / 3
        case .play, .pause, .headphones, .quit, .power, .chevron, .back, .more, .flow, .depth, .character, .speed, .gust, .dot,
             .thermometer, .droplet, .leaf, .bolt, .width, .stone, .gain, .whale, .dolphin, .comet, .beam, .orbit, .plasma: return nil
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
                IconLayer(icon: icon, color: NSColor(color), animating: animating && !Motion.reduced, struck: struck, lineWidth: lineWidth)
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
        strike.isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        if bounds.size != builtSize, let icon { build(icon) }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        if newSize != builtSize { needsLayout = true }
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        let scale = window?.backingScaleFactor ?? 2
        (strokes + [strike]).forEach { $0.contentsScale = scale }
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

    /// Builds the stroke layers already coloured: SwiftUI often configures the view before it has
    /// a size, so the layers are made later, in layout(), and must not wait for the next
    /// configure (a hover) to get their colour. They used to: the pictograms were invisible
    /// until hovered.
    private func build(_ icon: Icon) {
        builtSize = bounds.size
        strokes.forEach { $0.removeFromSuperlayer() }
        strokes = []
        guard bounds.width > 0 else { return }
        let k = bounds.width / 24
        let scale = window?.backingScaleFactor ?? 2
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        for s in icon.strokes(t: 0, level: 0.6) {
            let l = CAShapeLayer()
            l.contentsScale = scale
            l.strokeColor = color.cgColor
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
        strike.strokeColor = color.cgColor
        strike.contentsScale = scale
        layer?.addSublayer(strike)
        let wasRunning = running
        running = false
        if wasRunning { setRunning(true) }
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

/// The palette: greys on a dark panel. The accent belongs to the listener alone.
enum Ink {
    /// The popover's ground (the strips' shaders use the same value, PANEL).
    static let panel = Color(white: 0.078)
    /// Fields and hovered controls.
    static let raised = Color(white: 0.13)
    static let pressed = Color(white: 0.19)
    static let hairline = Color(white: 0.21)
    /// Primary text and active icons.
    static let paper = Color(white: 0.93)
    /// Secondary text, resting icons.
    static let graphite = Color(white: 0.62)
    /// Hints, disabled.
    static let ash = Color(white: 0.42)
    static let black = Color(white: 0)
    /// The user's accent (Accent.swift, violet by default): "you", and nothing else.
    static var accent: Color { Accent.color }
    /// The accent's old name, kept so existing code reads the current accent.
    static var violet: Color { accent }

    /// Coppice Mono (tools/font/coppice.py), bundled via ATSApplicationFontsPath. One face, four sizes:
    static func text(_ size: CGFloat = Size.body) -> Font { .custom("CoppiceMono-Regular", size: size) }
    static func italic(_ size: CGFloat = Size.body) -> Font { text(size) }

    enum Size {
        /// Hints and the spectrogram's scale.
        static let caption: CGFloat = 11
        /// Labels and values.
        static let body: CGFloat = 12
        /// The drawer's title and the selection.
        static let title: CGFloat = 13
        /// What the user writes.
        static let prompt: CGFloat = 14
    }
}

/// Spacing on a 4 pt grid.
enum Grid {
    static let x1: CGFloat = 4
    static let x2: CGFloat = 8
    static let x3: CGFloat = 12
    static let x4: CGFloat = 16
    static let x6: CGFloat = 24

    /// A button, a segmented switch and the prompt's inner buttons are 32 pt tall.
    static let control: CGFloat = 32
    /// Corner radii, one per depth: controls and fields 8, chips and menus 6, small plates and
    /// rows nested in a menu or field 4. A shape inset in another one takes the outer radius
    /// minus the inset, so the two curves stay concentric.
    static let radius: CGFloat = 8
    static let chipRadius: CGFloat = 6
    static let innerRadius: CGFloat = 4
    /// Sliders everywhere: a 2 pt rail; the knob 2 × 12 at rest, 3 × 16 under the pointer.
    static let rail: CGFloat = 2
    static let knob = CGSize(width: 2, height: 12)
    static let knobActive = CGSize(width: 3, height: 16)
    /// Keyboard focus: a 1.5 pt ring drawn 2 pt outside the control.
    static let focusRing: CGFloat = 1.5
    static let focusGap: CGFloat = 2
}

/// System "Reduce motion": decorative animation stops (icons, the menu bar wave, the cursor).
enum Motion {
    static var reduced: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
}
