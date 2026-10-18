import CoreGraphics

/// Indexed colour palette. Cells store a UInt8 index; ramps are 16 steps each.
enum Pal {
    static let none: UInt8 = 0
    static let text: UInt8 = 1
    static let dim: UInt8 = 2
    static let faint: UInt8 = 3
    static let bright: UInt8 = 4
    static let amber: UInt8 = 5
    static let red: UInt8 = 6
    static let cyan: UInt8 = 7
    static let pink: UInt8 = 8
    static let trunk: UInt8 = 9
    static let sand: UInt8 = 10
    static let sun: UInt8 = 11
    static let hoverBg: UInt8 = 12
    static let activeBg: UInt8 = 13
    static let panelBg: UInt8 = 14

    static let heatBase: UInt8 = 32
    static let greenBase: UInt8 = 48
    static let blueBase: UInt8 = 64
    static let skyBase: UInt8 = 80
    static let titleBase: UInt8 = 96

    static let background = CGColor(srgbRed: 0.018, green: 0.034, blue: 0.028, alpha: 1)

    static func ramp(_ base: UInt8, _ v: Float) -> UInt8 {
        base + UInt8(max(0, min(15, Int(v * 15.99))))
    }

    static func heat(_ v: Float) -> UInt8 { ramp(heatBase, v) }

    static let colors: [CGColor] = {
        var c = [CGColor](repeating: CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 0), count: 112)
        func rgb(_ r: Double, _ g: Double, _ b: Double) -> CGColor {
            CGColor(srgbRed: r, green: g, blue: b, alpha: 1)
        }
        c[Int(text)] = rgb(0.55, 1.00, 0.66)
        c[Int(dim)] = rgb(0.30, 0.62, 0.40)
        c[Int(faint)] = rgb(0.16, 0.32, 0.22)
        c[Int(bright)] = rgb(0.92, 1.00, 0.95)
        c[Int(amber)] = rgb(1.00, 0.72, 0.25)
        c[Int(red)] = rgb(1.00, 0.33, 0.33)
        c[Int(cyan)] = rgb(0.40, 0.90, 1.00)
        c[Int(pink)] = rgb(1.00, 0.45, 0.75)
        c[Int(trunk)] = rgb(0.55, 0.38, 0.22)
        c[Int(sand)] = rgb(0.62, 0.55, 0.38)
        c[Int(sun)] = rgb(1.00, 0.85, 0.35)
        c[Int(hoverBg)] = rgb(0.10, 0.24, 0.15)
        c[Int(activeBg)] = rgb(0.18, 0.45, 0.26)
        c[Int(panelBg)] = rgb(0.04, 0.08, 0.06)

        func fill(_ base: UInt8, _ stops: [(Double, Double, Double)]) {
            for i in 0..<16 {
                let x = Double(i) / 15 * Double(stops.count - 1)
                let k = min(Int(x), stops.count - 2)
                let f = x - Double(k)
                let a = stops[k], b = stops[k + 1]
                c[Int(base) + i] = rgb(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
            }
        }
        // Inferno-like heat map for the spectrogram.
        fill(heatBase, [(0.10, 0.05, 0.22), (0.38, 0.07, 0.48), (0.76, 0.16, 0.38),
                        (0.97, 0.46, 0.10), (0.99, 0.85, 0.30), (1.00, 1.00, 0.90)])
        fill(greenBase, [(0.06, 0.20, 0.10), (0.14, 0.42, 0.17), (0.42, 0.78, 0.30), (0.82, 0.95, 0.52)])
        fill(blueBase, [(0.06, 0.16, 0.36), (0.10, 0.40, 0.70), (0.35, 0.75, 0.95), (0.86, 0.97, 1.00)])
        fill(skyBase, [(0.11, 0.14, 0.21), (0.34, 0.41, 0.54), (0.76, 0.81, 0.89)])
        fill(titleBase, [(0.40, 0.90, 1.00), (0.55, 1.00, 0.66), (1.00, 0.72, 0.25),
                         (1.00, 0.45, 0.75), (0.40, 0.90, 1.00)])
        return c
    }()
}

/// A character-cell framebuffer: one Unicode scalar + fg/bg palette index per cell.
final class AsciiGrid {
    let cols: Int
    let rows: Int
    private(set) var glyphs: [UInt32]
    private(set) var fg: [UInt8]
    private(set) var bg: [UInt8]

    init(cols: Int, rows: Int) {
        self.cols = cols
        self.rows = rows
        glyphs = Array(repeating: 32, count: cols * rows)
        fg = Array(repeating: 0, count: cols * rows)
        bg = Array(repeating: 0, count: cols * rows)
    }

    func clear() {
        for i in glyphs.indices {
            glyphs[i] = 32
            fg[i] = 0
            bg[i] = 0
        }
    }

    @inline(__always)
    func put(_ x: Int, _ y: Int, raw: UInt32, _ color: UInt8) {
        guard x >= 0, y >= 0, x < cols, y < rows else { return }
        let i = y * cols + x
        glyphs[i] = raw
        fg[i] = color
    }

    @inline(__always)
    func put(_ x: Int, _ y: Int, _ s: Unicode.Scalar, _ color: UInt8) {
        put(x, y, raw: s.value, color)
    }

    /// Writes a string, returns the column after its last character.
    @discardableResult
    func text(_ x: Int, _ y: Int, _ s: String, _ color: UInt8) -> Int {
        var cx = x
        for u in s.unicodeScalars {
            put(cx, y, u, color)
            cx += 1
        }
        return cx
    }

    func fillBg(_ x: Int, _ y: Int, _ w: Int, _ h: Int, _ color: UInt8) {
        for yy in max(0, y)..<min(rows, y + h) {
            for xx in max(0, x)..<min(cols, x + w) {
                bg[yy * cols + xx] = color
            }
        }
    }

    func box(_ x: Int, _ y: Int, _ w: Int, _ h: Int, color: UInt8,
             title: String? = nil, titleColor: UInt8 = Pal.text, right: String? = nil, rightColor: UInt8 = Pal.dim) {
        guard w >= 2, h >= 2 else { return }
        for xx in (x + 1)..<(x + w - 1) {
            put(xx, y, "─", color)
            put(xx, y + h - 1, "─", color)
        }
        for yy in (y + 1)..<(y + h - 1) {
            put(x, yy, "│", color)
            put(x + w - 1, yy, "│", color)
        }
        put(x, y, "╭", color)
        put(x + w - 1, y, "╮", color)
        put(x, y + h - 1, "╰", color)
        put(x + w - 1, y + h - 1, "╯", color)
        if let title { text(x + 2, y, title, titleColor) }
        if let right { text(x + w - 2 - right.unicodeScalars.count, y, right, rightColor) }
    }
}

/// 2x4 sub-cell "pixels" using Unicode Braille patterns (U+2800..U+28FF).
struct BrailleCanvas {
    let w: Int
    let h: Int
    private var bits: [UInt8]
    private var color: [UInt8]
    private var prio: [UInt8]

    var pixelWidth: Int { w * 2 }
    var pixelHeight: Int { h * 4 }

    private static let dotBit: [[UInt8]] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]]

    init(w: Int, h: Int) {
        self.w = w
        self.h = h
        bits = Array(repeating: 0, count: w * h)
        color = Array(repeating: 0, count: w * h)
        prio = Array(repeating: 0, count: w * h)
    }

    mutating func clear() {
        for i in bits.indices {
            bits[i] = 0
            color[i] = 0
            prio[i] = 0
        }
    }

    /// Lights a dot. A cell has one colour; the highest-priority dot in it decides.
    mutating func set(_ px: Int, _ py: Int, _ c: UInt8, prio p: UInt8 = 1) {
        guard px >= 0, py >= 0, px < pixelWidth, py < pixelHeight else { return }
        let i = (py >> 2) * w + (px >> 1)
        bits[i] |= Self.dotBit[py & 3][px & 1]
        if p >= prio[i] {
            prio[i] = p
            color[i] = c
        }
    }

    func blit(into g: AsciiGrid, x: Int, y: Int) {
        for cy in 0..<h {
            for cx in 0..<w {
                let i = cy * w + cx
                if bits[i] != 0 {
                    g.put(x + cx, y + cy, raw: 0x2800 + UInt32(bits[i]), color[i])
                }
            }
        }
    }
}

// MARK: - Noise & randomness for the animations

@inline(__always)
private func hash2(_ x: Int, _ y: Int) -> Double {
    var h = UInt64(bitPattern: Int64(x &* 374_761_393 &+ y &* 668_265_263))
    h = (h ^ (h >> 13)) &* 1_274_126_177
    h ^= h >> 16
    return Double(h & 0xFFFF) / 65535.0
}

/// Smooth value noise in 0..1.
func vnoise(_ x: Double, _ y: Double) -> Double {
    let fx = floor(x), fy = floor(y)
    let xi = Int(fx), yi = Int(fy)
    let tx = x - fx, ty = y - fy
    let sx = tx * tx * (3 - 2 * tx)
    let sy = ty * ty * (3 - 2 * ty)
    let a = hash2(xi, yi), b = hash2(xi + 1, yi)
    let c = hash2(xi, yi + 1), d = hash2(xi + 1, yi + 1)
    return (a + (b - a) * sx) + ((c + (d - c) * sx) - (a + (b - a) * sx)) * sy
}

func fbm(_ x: Double, _ y: Double) -> Double {
    0.6 * vnoise(x, y) + 0.3 * vnoise(2 * x + 5.2, 2 * y + 1.3) + 0.1 * vnoise(4 * x - 3.1, 4 * y + 7.7)
}

struct LCG {
    private var state: UInt64
    init(seed: UInt64) { state = seed &* 6_364_136_223_846_793_005 &+ 1 }
    mutating func next() -> Double {
        state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
        return Double(state >> 11) / Double(1 << 53)
    }
    mutating func range(_ a: Double, _ b: Double) -> Double { a + (b - a) * next() }
    mutating func int(_ n: Int) -> Int { min(n - 1, Int(next() * Double(n))) }
}
