import CoreGraphics
import Foundation
import ImageIO
import simd

// The onboarding's art: original anime portraits (Resources/Onboarding, generated for the app)
// drawn through an ordered dither: Bayer 8×8 on the luminance, one ink per pixel, a crisp grid of
// 2 pt cells. Lit areas come out in paper, the shadow side in red, lines and pupils in the void.
// Plain Swift and CoreGraphics, no UI: OnboardingView draws the bitmaps, and frames can be
// rendered outside the app.

/// Head orientation in radians, engine convention (HeadTracker): yaw + = turned left,
/// pitch + = looking up, roll + = right ear down. The head on screen is the listener's mirror
/// image: turning left turns its nose to the left of the screen.
struct HeadPose: Equatable {
    var yaw = 0.0
    var pitch = 0.0
    var roll = 0.0

    static func mix(_ a: HeadPose, _ b: HeadPose, _ t: Double) -> HeadPose {
        HeadPose(yaw: a.yaw + (b.yaw - a.yaw) * t, pitch: a.pitch + (b.pitch - a.pitch) * t, roll: a.roll + (b.roll - a.roll) * t)
    }

    /// The idle turn of the intro: a slow look left and right with a little nod and tilt.
    static func idle(_ t: Double, amount: Double = 1) -> HeadPose {
        HeadPose(yaw: amount * (0.62 * sin(t * 0.55) + 0.08 * sin(t * 1.37)),
                 pitch: amount * (0.11 * sin(t * 0.41 + 1.1) - 0.03),
                 roll: amount * 0.09 * sin(t * 0.55 + 0.6))
    }
}

/// When the eyes blink: every 2.4–5.4 s, now and then twice. 1 = open.
enum Blink {
    static func openness(at t: Double, seed: Double = 0) -> Double {
        var start = 1.2 + seed, i = 0.0, last = -10.0, twice = false
        while start <= t {
            last = start
            twice = fract(sin((i + seed) * 7.13) * 917.3) > 0.72
            start += 2.4 + 3.0 * fract(sin((i + seed) * 12.9898) * 43758.5453)
            i += 1
        }
        let a = shut(t - last)
        return twice ? min(a, shut(t - last - 0.27)) : a
    }

    private static func shut(_ dt: Double) -> Double {
        guard dt >= 0, dt < 0.17 else { return 1 }
        let x = dt / 0.17
        // Fast down, slower up.
        return x < 0.4 ? 1 - x / 0.4 : (x - 0.4) / 0.6
    }

    static func fract(_ x: Double) -> Double { x - x.rounded(.down) }
}


// MARK: - Sources

/// One portrait as luminance and coverage, row-major, top row first.
struct DitherSource {
    let width: Int
    let height: Int
    let lum: [Float]
    let alpha: [Float]

    init?(url: URL) {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else { return nil }
        let w = img.width, h = img.height
        var px = [UInt8](repeating: 0, count: w * h * 4)
        guard let ctx = CGContext(data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
        var lum = [Float](repeating: 0, count: w * h), alpha = [Float](repeating: 0, count: w * h)
        for i in 0..<(w * h) {
            let a = Float(px[i * 4 + 3]) / 255
            alpha[i] = a
            guard a > 0 else { continue }
            let r = Float(px[i * 4]) / 255 / a, g = Float(px[i * 4 + 1]) / 255 / a, b = Float(px[i * 4 + 2]) / 255 / a
            lum[i] = 0.2126 * r + 0.7152 * g + 0.0722 * b
        }
        width = w; height = h
        self.lum = lum; self.alpha = alpha
    }

    /// A cell's worth of the image around (u, v), `r` its half-width in u: four taps, half their
    /// mean and half their darkest, so thin line art survives the downsampling.
    func cell(_ u: Double, _ v: Double, _ r: Double) -> (Float, Float) {
        let a = sample(u - r, v - r), b = sample(u + r, v - r), c = sample(u - r, v + r), d = sample(u + r, v + r)
        let cov = (a.1 + b.1 + c.1 + d.1) / 4
        var lums: [Float] = []
        for t in [a, b, c, d] where t.1 > 0.3 { lums.append(t.0) }
        guard let lo = lums.min() else { return (0, cov) }
        let mean = lums.reduce(0, +) / Float(lums.count)
        return (0.5 * mean + 0.5 * lo, cov)
    }

    /// Bilinear sample at (u, v) in 0…1 (top-left origin): (luminance, coverage).
    func sample(_ u: Double, _ v: Double) -> (Float, Float) {
        let x = u * Double(width) - 0.5, y = v * Double(height) - 0.5
        let x0 = Int(x.rounded(.down)), y0 = Int(y.rounded(.down))
        let fx = Float(x - Double(x0)), fy = Float(y - Double(y0))
        func at(_ xi: Int, _ yi: Int) -> (Float, Float) {
            guard xi >= 0, yi >= 0, xi < width, yi < height else { return (0, 0) }
            let i = yi * width + xi
            return (lum[i] * alpha[i], alpha[i])
        }
        let a = at(x0, y0), b = at(x0 + 1, y0), c = at(x0, y0 + 1), d = at(x0 + 1, y0 + 1)
        let al = (a.1 * (1 - fx) + b.1 * fx) * (1 - fy) + (c.1 * (1 - fx) + d.1 * fx) * fy
        let lm = (a.0 * (1 - fx) + b.0 * fx) * (1 - fy) + (c.0 * (1 - fx) + d.0 * fx) * fy
        return (al > 0.001 ? lm / al : 0, al)
    }
}

/// The portraits, loaded once: from the app bundle (Contents/Resources/Onboarding), or from the
/// source tree when run from .build.
enum Portraits {
    static let directory: URL = {
        if let url = Bundle.main.url(forResource: "Onboarding", withExtension: nil) { return url }
        return URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../Resources/Onboarding").standardizedFileURL
    }()

    static func load(_ name: String) -> DitherSource? { DitherSource(url: directory.appendingPathComponent(name + ".png")) }

    static let front = load("front")
    /// Face turned towards the left of the image (the listener turned left, mirrored).
    static let turnLeft = load("turn_left")
    static let turnRight = load("turn_right")
    static let profileRight = load("profile_right")
    static let eye = load("eye")

    /// Where the visible earbud is in each view (u, v of the image).
    static let podFront = SIMD2<Double>(0.67, 0.47)
    static let podTurnLeft = SIMD2<Double>(0.573, 0.44)
    static let podTurnRight = SIMD2<Double>(0.427, 0.44)
    static let podProfile = SIMD2<Double>(0.467, 0.44)
    /// Her eye in the profile view, for the line of sight.
    static let eyeProfile = SIMD2<Double>(0.707, 0.36)
}

// MARK: - The dither

/// The inks a dithered pixel can take (the panel's paper, the shadow red, the listener's violet).
enum DitherInk: UInt8 { case none, paper, red, violet }

struct DitherFrame {
    let cols: Int
    let rows: Int
    var ink: [DitherInk]

    init(cols: Int, rows: Int) {
        self.cols = cols; self.rows = rows
        ink = Array(repeating: .none, count: cols * rows)
    }

    /// RGBA, one pixel per cell, for drawing scaled up without interpolation.
    func image(paper: (UInt8, UInt8, UInt8), red: (UInt8, UInt8, UInt8), violet: (UInt8, UInt8, UInt8)) -> CGImage? {
        var px = [UInt8](repeating: 0, count: cols * rows * 4)
        for i in 0..<(cols * rows) {
            let c: (UInt8, UInt8, UInt8)
            switch ink[i] {
            case .none: continue
            case .paper: c = paper
            case .red: c = red
            case .violet: c = violet
            }
            px[i * 4] = c.0; px[i * 4 + 1] = c.1; px[i * 4 + 2] = c.2; px[i * 4 + 3] = 255
        }
        let data = Data(px) as CFData
        guard let provider = CGDataProvider(data: data) else { return nil }
        return CGImage(width: cols, height: rows, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: cols * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}

enum Dither {
    /// The 8×8 Bayer matrix, thresholds in (0, 1).
    static let bayer: [Float] = {
        let m: [Int] = [0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26,
                        12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22,
                        3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25,
                        15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21]
        return m.map { (Float($0) + 0.5) / 64 }
    }()

    /// The tone mapping: `t` is the lit luminance (0 black … 1 white). Lit: paper, denser as it
    /// brightens; the shadow band: an even red screen; line art and pupils: nothing.
    /// `shade` (0…1) is how far into the shadow side the pixel is: there the screen turns red
    /// (a dithered terminator, `alt` is a second, offset threshold for it).
    @inline(__always)
    static func ink(_ t: Float, _ threshold: Float, shade: Float, alt: Float) -> DitherInk {
        let d: Float
        if t > 0.5 { d = 0.3 + 0.7 * min(1, (t - 0.5) / 0.4) } else if t > 0.16 { d = 0.12 + 0.18 * (t - 0.16) / 0.34 } else { return .none }
        guard threshold < d else { return .none }
        return alt < shade ? .red : .paper
    }

    @inline(__always)
    static func alt(_ x: Int, _ y: Int) -> Float { bayer[((y + 3) & 7) * 8 + ((x + 5) & 7)] }

    /// Light across the image: full on the left, falling into shadow on the right (a hard side
    /// light, the series' flat overexposure against deep shade).
    @inline(__always)
    static func light(_ u: Double) -> Float { Float(1.12 - 0.3 * Double(shade(u))) }

    /// 0 on the lit side, 1 deep in the shadow side.
    @inline(__always)
    static func shade(_ u: Double) -> Float {
        let s = max(0, min(1, (u - 0.5) / 0.14))
        return Float(s * s * (3 - 2 * s))
    }
}

/// What the portrait does in one frame.
struct PortraitLook {
    var pose = HeadPose()
    /// The earbud takes the violet (the listener's head is linked).
    var linked = false
}

/// The head: the front and two three-quarter views blended by the yaw, shifted a little by the
/// pitch (parallax), rotated by the roll, dithered.
enum PortraitDither {
    /// Renders `cols × rows` cells showing the portrait square in the middle (`size` cells).
    /// Returns the frame and where the earbud is, in cells.
    /// In profile she faces the screen on the right, and it lights her face: the shadow side
    /// flips to the left.
    static func head(_ look: PortraitLook, cols: Int, rows: Int, size: Int, profile: Bool = false) -> (DitherFrame, SIMD2<Double>) {
        var frame = DitherFrame(cols: cols, rows: rows)
        // Mirror image: turning left (yaw +) turns her face towards the left of the screen.
        let p = max(-1, min(1, look.pose.yaw / 0.6))
        var views: [(DitherSource, Float, SIMD2<Double>)] = []
        if profile {
            if let s = Portraits.profileRight { views = [(s, 1, Portraits.podProfile)] }
        } else if let f = Portraits.front {
            let side = p > 0 ? Portraits.turnLeft : Portraits.turnRight
            let pod = p > 0 ? Portraits.podTurnLeft : Portraits.podTurnRight
            let w = Float(abs(p))
            // A short crossfade in the middle of the turn: each view holds, then hands over
            // quickly (a long blend shows two faces at once).
            let e = max(0, min(1, (w - 0.38) / 0.24))
            let k = e * e * (3 - 2 * e)
            views = [(f, 1 - k, Portraits.podFront)]
            if let side, k > 0.001 { views.append((side, k, pod)) }
        }
        guard !views.isEmpty else { return (frame, SIMD2(Double(cols) / 2, Double(rows) / 2)) }

        let ox = Double(cols - size) / 2, oy = Double(rows - size) / 2
        let shiftU = -look.pose.yaw * 0.035, shiftV = -look.pose.pitch * 0.09
        let c = cos(-look.pose.roll), s = sin(-look.pose.roll)
        // Rotate about the neck.
        let pu = 0.5, pv = 0.85
        for y in 0..<rows {
            for x in 0..<cols {
                let u0 = (Double(x) + 0.5 - ox) / Double(size), v0 = (Double(y) + 0.5 - oy) / Double(size)
                let du = u0 - pu, dv = v0 - pv
                let u = pu + du * c - dv * s - shiftU, v = pv + du * s + dv * c - shiftV
                var lum: Float = 0, cov: Float = 0
                let r = 0.35 / Double(size)
                for (src, w, _) in views {
                    let (l, a) = src.cell(u, v, r)
                    lum += l * a * w
                    cov += a * w
                }
                guard cov > 0.35 else { continue }
                let lu = profile ? 0.94 - u0 : u0 - 0.04
                let t = lum / cov * Dither.light(lu)
                let th = Dither.bayer[(y & 7) * 8 + (x & 7)]
                // The image's sides and bottom dissolve into the void (no cut-off shoulders).
                let edge = min(1, min(u0, 1 - u0) / 0.14, (1 - v0) / 0.2)
                guard th < Float(edge) * 1.05 else { continue }
                frame.ink[y * cols + x] = Dither.ink(t, th, shade: Dither.shade(lu), alt: Dither.alt(x, y))
            }
        }
        // The earbud: where the blended views put it, back through the same transform.
        var pod = SIMD2<Double>(0, 0)
        var total = 0.0
        for (_, w, pp) in views { pod += pp * Double(w); total += Double(w) }
        pod /= max(total, 0.001)
        let du = pod.x + shiftU - pu, dv = pod.y + shiftV - pv
        let pu0 = pu + du * c + dv * s, pv0 = pv - du * s + dv * c
        let cell = SIMD2(ox + pu0 * Double(size), oy + pv0 * Double(size))
        if look.linked {
            // The earbud in violet: the paper pixels around it.
            let r = Double(size) * 0.035
            for y in max(0, Int(cell.y - r))...min(rows - 1, Int(cell.y + r)) {
                for x in max(0, Int(cell.x - r))...min(cols - 1, Int(cell.x + r)) {
                    let dx = Double(x) - cell.x, dy = Double(y) - cell.y
                    if dx * dx + dy * dy <= r * r, frame.ink[y * cols + x] != .none { frame.ink[y * cols + x] = .violet }
                }
            }
        }
        return (frame, cell)
    }
}

struct EyeShot {
    /// 1 open, 0 shut.
    var open = 1.0
    /// Where the iris looks: x + right, y + up, about -1…1.
    var gaze = SIMD2<Double>(0, 0)
    /// The iris takes the violet (locked on).
    var locked = false
}

/// The close-up of her eye: a band through the eye image, dithered the same way. The iris is
/// moved inside the opening by a small displacement field, a blink closes the opening from the
/// top, and on the lock the iris turns violet.
enum EyeDither {
    // In the eye image (u, v): the iris centre and radius, the opening.
    static let iris = SIMD2<Double>(0.49, 0.52)
    static let irisR = 0.17
    static let opening = (u0: 0.13, u1: 0.73, top: 0.36, bottom: 0.66)

    /// `cols × rows` cells showing the image `size` cells wide, its eye at `centre` (cells).
    static func render(_ look: EyeShot, cols: Int, rows: Int, size: Int, centre: SIMD2<Double>) -> DitherFrame {
        var frame = DitherFrame(cols: cols, rows: rows)
        guard let src = Portraits.eye else { return frame }
        let s = Double(size)
        let ou = centre.x / s - iris.x, ov = centre.y / s - iris.y
        let g = SIMD2(max(-1, min(1, look.gaze.x)) * 0.11, -max(-1, min(1, look.gaze.y)) * 0.07)
        let midV = (opening.top + opening.bottom) / 2, halfV = (opening.bottom - opening.top) / 2
        let lid = opening.top + (1 - max(0, min(1, look.open))) * (opening.bottom - opening.top) * 0.85
        for y in 0..<rows {
            for x in 0..<cols {
                var u = (Double(x) + 0.5) / s - ou, v = (Double(y) + 0.5) / s - ov
                // Inside the opening: shift what is drawn by the gaze, fading to 0 at its rim.
                let eu = (u - (opening.u0 + opening.u1) / 2) / ((opening.u1 - opening.u0) / 2)
                let ev = (v - midV) / halfV
                let inside = eu * eu + ev * ev
                var shut = false
                if inside < 1 {
                    let k = pow(1 - inside, 0.6)
                    u -= g.x * k; v -= g.y * k
                    if v < lid, look.open < 0.98 { shut = true }
                }
                // Edges of the band fall off into the void.
                let fx = Double(x) / Double(cols), fy = Double(y) / Double(rows)
                let edge = Float(min(1, min(fx, 1 - fx) / 0.14) * min(1, min(fy, 1 - fy) / 0.2))
                var (l, a) = src.cell(u, v, 0.35 / s)
                if shut { l = 0.9 } // the lid: skin
                guard a > 0.3 else { continue }
                let th = Dither.bayer[(y & 7) * 8 + (x & 7)]
                // The band dissolves into the void at its edges.
                guard th < edge * 1.05 else { continue }
                // Lift the iris's browns out of the void (the pupil stays dark).
                let t = pow(l, 0.62) * Dither.light(fx)
                var ink = Dither.ink(t, th, shade: Dither.shade(fx), alt: Dither.alt(x, y))
                if look.locked, ink != .none, !shut {
                    let du = u - iris.x, dv = v - iris.y
                    if du * du + dv * dv < irisR * irisR { ink = .violet }
                }
                frame.ink[y * cols + x] = ink
            }
        }
        return frame
    }
}
