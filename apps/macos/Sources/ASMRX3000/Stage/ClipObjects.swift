import Foundation
import simd

/// The things a clip scene's sounds are drawn as, besides the animals and rocks of
/// `ClipModels`: small low-poly models in the stage's grey stone, each about two units across
/// (x forward, y left, z up, the sounding point at the origin) and scaled on stage by
/// `ClipActors`. Light parts are stone, dark ones (iron, windows, silhouettes) wood, the
/// in-between section grey.
enum Prop: CaseIterable {
    // Under water
    case reef, coral, fish, rainSurface
    // Weather, anywhere (matched by the procedural sound model)
    case wind, rain, sea, bolt, brook
    // Body scan
    case bead, singingBowl, tingsha
    /// A bed that is no object (a hum, a room tone): a small gimbal of two hoops.
    case marker
}

private func v(_ x: Float, _ y: Float, _ z: Float) -> SIMD3<Float> { SIMD3(x, y, z) }
private func v2(_ x: Float, _ y: Float) -> SIMD2<Float> { SIMD2(x, y) }
private let LIGHT = Part.stone, DARK = Part.wood, MID = Part.section
private let X = SIMD3<Float>(1, 0, 0), Y = SIMD3<Float>(0, 1, 0), Z = SIMD3<Float>(0, 0, 1)

/// The meshes, built once on first use.
enum PropMeshes {
    private static func make(_ f: (inout MeshBuilder) -> Void) -> Mesh? {
        var b = MeshBuilder()
        f(&b)
        return b.verts.isEmpty ? nil : b.build()
    }

    private static func builder(_ f: (inout MeshBuilder) -> Void) -> MeshBuilder {
        var b = MeshBuilder()
        f(&b)
        return b
    }

    static func mesh(_ p: Prop) -> Mesh? {
        switch p {
        case .reef: return reef
        case .coral: return coral
        case .fish: return fish
        case .rainSurface: return droplets
        case .wind: return wind
        case .rain: return rain
        case .sea: return crest
        case .bolt: return bolt
        case .bead: return bead
        case .singingBowl: return singingBowl
        case .tingsha: return tingsha
        case .brook: return brook
        case .marker: return marker
        }
    }

    static let reef = make { b in
        b.rock(seed: 41, bumps: 0.25, squash: v(1.0, 0.72, 0.24), part: MID)
        var rng = SeededGenerator(77)
        for k in 0..<9 {
            let a = Float(k) * 2.4
            let r = 0.15 + 0.6 * rng.next()
            let base = v(r * cos(a), r * 0.7 * sin(a), 0.15)
            coralHead(&b, base, scale: 0.35 + 0.25 * rng.next(), kind: k % 3, rng: &rng)
        }
    }

    static let coral = make { b in
        b.rock(seed: 9, bumps: 0.3, squash: v(0.6, 0.5, 0.25), part: MID)
        var rng = SeededGenerator(13)
        coralHead(&b, v(0, 0, 0.12), scale: 0.9, kind: 0, rng: &rng)
        coralHead(&b, v(0.3, 0.2, 0.06), scale: 0.6, kind: 1, rng: &rng)
        coralHead(&b, v(-0.25, -0.15, 0.08), scale: 0.6, kind: 2, rng: &rng)
    }

    /// Branching coral (0), a brain coral (1) or a sea fan (2) at `c`.
    private static func coralHead(_ b: inout MeshBuilder, _ c: SIMD3<Float>, scale s: Float, kind: Int, rng: inout SeededGenerator) {
        switch kind {
        case 0:
            for i in 0..<5 {
                let a = Float(i) * 1.26 + rng.next()
                let tip = c + v(0.25 * cos(a), 0.25 * sin(a), 0.55) * s
                let mid = c + v(0.08 * cos(a), 0.08 * sin(a), 0.28) * s
                b.rod(c, mid, 0.06 * s, 0.05 * s, segs: 5, part: LIGHT)
                b.rod(mid, tip, 0.05 * s, 0.025 * s, segs: 5, part: LIGHT)
                b.rod(mid, mid + v(0.2 * cos(a + 0.8), 0.2 * sin(a + 0.8), 0.18) * s, 0.035 * s, 0.02 * s, segs: 4, part: LIGHT)
            }
        case 1:
            b.ball(c + v(0, 0, 0.12) * s, 0.26 * s, part: LIGHT, squash: v(1, 1, 0.7))
        default:
            let u = v(cos(rng.next() * 3), sin(rng.next() * 3), 0)
            b.sheet([v2(0, 0), v2(-0.3, 0.35), v2(-0.2, 0.6), v2(0.05, 0.68), v2(0.3, 0.55), v2(0.32, 0.3)], 0.02 * s,
                    at: c, u: u * s, v: Z * s, part: LIGHT)
        }
    }

    static let fishParts: MeshBuilder = builder { b in
        b.lathe([(0.5, 0.005, 0.005, 0), (0.42, 0.06, 0.08, 0), (0.2, 0.1, 0.15, 0.01), (-0.15, 0.07, 0.1, 0.01), (-0.32, 0.02, 0.03, 0.01)],
                segs: 8, part: LIGHT)
        b.sheet([v2(-0.3, 0), v2(-0.5, 0.16), v2(-0.45, 0), v2(-0.5, -0.16)], 0.015, at: v(0, 0, 0.01), u: X, v: Z, part: LIGHT)
        b.sheet([v2(0.15, 0.12), v2(-0.05, 0.24), v2(-0.15, 0.1)], 0.012, at: .zero, u: X, v: Z, part: LIGHT)
        b.ball(v(0.38, 0.05, 0.03), 0.02, part: DARK)
        b.ball(v(0.38, -0.05, 0.03), 0.02, part: DARK)
    }

    /// A few small fish together.
    static let fish = make { b in
        for (o, s) in [(v(0.4, 0.1, 0.05), Float(1.0)), (v(-0.1, 0.45, -0.1), 0.85), (v(-0.3, -0.35, 0.12), 0.9), (v(-0.75, 0.05, -0.05), 0.75)] {
            b.add(fishParts, M4.move(o) * M4.rotZ(0.15 * o.y) * M4.scale(s))
        }
    }

    /// Drops falling in a column (z 0...1.2, spaced 0.3: slid down by up to 0.3 they repeat).
    static let droplets = make { b in
        var rng = SeededGenerator(5)
        for k in 0..<10 {
            let z = 0.12 * Float(k)
            let a = rng.next() * 6.28, r = 0.15 + 0.3 * rng.next()
            b.ball(v(r * cos(a), r * sin(a), z), 0.045, part: LIGHT, squash: v(0.7, 0.7, 1.6))
        }
    }


    /// Three helical strands along x; turned about x they seem to stream.
    static let wind = make { b in
        for s in 0..<3 {
            var pts: [SIMD3<Float>] = []
            let r0: Float = [0.1, 0.18, 0.13][s], off = Float(s) * 2.1, z0: Float = [0, 0.18, -0.16][s]
            for i in 0...28 {
                let x = -1 + 2 * Float(i) / 28
                let env = sin(.pi * Float(i) / 28)
                let a = x * 7 + off
                pts.append(v(x, r0 * env * cos(a), z0 + r0 * env * sin(a)))
            }
            b.path(pts, 0.012, 0.022, segs: 4, part: LIGHT)
        }
    }

    /// Slanting streaks in a column, repeating every 0.4 in z (slid down by 0.4 they match).
    static let rain = make { b in
        var rng = SeededGenerator(19)
        for _ in 0..<7 {
            let a = rng.next() * 6.28, r = 0.4 * rng.next().squareRoot()
            let x = r * cos(a), y = r * sin(a)
            let z0 = 0.4 * rng.next()
            for j in 0..<4 {
                let z = -0.8 + z0 + 0.4 * Float(j)
                b.rod(v(x, y, z), v(x + 0.04, y + 0.02, z - 0.18), 0.018, segs: 4, part: LIGHT)
            }
        }
    }

    /// A wave crest along x, with a lip of foam.
    static let crest = make { b in
        b.lathe([(-1.0, 0.02, 0.01, 0), (-0.7, 0.16, 0.06, 0.03), (0, 0.22, 0.12, 0.06), (0.7, 0.16, 0.06, 0.03), (1.0, 0.02, 0.01, 0)],
                segs: 8, part: Part.water)
        for i in 0..<9 {
            let x = -0.72 + 0.18 * Float(i)
            b.ball(v(x, 0.04, 0.15 * (1 - x * x * 0.8)), 0.045, part: LIGHT)
        }
    }

    /// A lightning glyph.
    static let bolt = make { b in
        b.path([v(-0.1, 0, 1.0), v(0.15, 0, 0.25), v(-0.15, 0, 0.12), v(0.08, 0, -0.95)], 0.07, 0.025, segs: 5, part: LIGHT)
        b.rod(v(0.15, 0, 0.25), v(0.4, 0, -0.1), 0.035, 0.012, segs: 4, part: LIGHT)
    }

    /// A flat hoop of radius 1 (the booms' rings).
    static let hoop = make { $0.hoop(1, 0.035, segs: 32, part: LIGHT) }

    /// A reach of the mountain stream along x: stones in it and the water's riffles between them.
    static let brook = make { b in
        var rng = SeededGenerator(0xB0)
        for _ in 0..<6 {
            var r = MeshBuilder()
            let sz = 0.1 + 0.14 * rng.next()
            r.rock(seed: UInt64(1 + rng.next() * 90), bumps: 0.14, squash: v(sz * 1.3, sz, sz * 0.6), part: MID)
            b.add(r, M4.move(v(-0.9 + 1.8 * rng.next(), -0.28 + 0.56 * rng.next(), 0)))
        }
        for k in 0..<3 {
            let y = -0.2 + 0.2 * Float(k)
            var pts: [SIMD3<Float>] = []
            for i in 0...12 { let x = -1 + 2 * Float(i) / 12; pts.append(v(x, y + 0.05 * sin(5 * x + Float(k) * 1.9), 0.04)) }
            b.path(pts, 0.012, segs: 4, part: Part.water)
        }
    }

    /// A small bead (`beadGlow` is its see-through halo).
    static let bead = make { b in
        b.turn((0...8).map { i -> SIMD2<Float> in
            let a = -Float.pi / 2 + Float.pi * Float(i) / 8
            return v2(cos(a) * 0.55, sin(a) * 0.55)
        }, segs: 12, part: LIGHT)
    }

    /// A glow about the bead: a shell and a tilted hoop.
    static let beadGlow = make { b in
        b.ball(.zero, 0.95, part: LIGHT)
        b.hoop(1.3, 0.05, segs: 24, part: LIGHT, at: M4.rotX(0.5))
    }

    /// A singing bowl on its ring cushion, the striker beside it.
    static let singingBowl = make { b in
        b.turn([v2(0, -0.3), v2(0.25, -0.3), v2(0.45, -0.12), v2(0.5, 0.15), v2(0.47, 0.17), v2(0.42, -0.08), v2(0.2, -0.25), v2(0, -0.25)],
               segs: 18, part: LIGHT)
        b.hoop(0.3, 0.1, segs: 16, part: MID, at: M4.move(v(0, 0, -0.38)))
        b.rod(v(0.35, 0.45, -0.42), v(-0.25, 0.75, -0.42), 0.045, segs: 6, part: MID)
    }

    /// Tingsha: two small cymbals on a cord.
    static let tingsha = make { b in
        for s: Float in [1, -1] {
            b.turn([v2(0, 0.1), v2(0.12, 0.09), v2(0.28, 0.02), v2(0.32, -0.02), v2(0, -0.02)], segs: 14, part: LIGHT,
                   at: M4.move(v(0, 0.3 * s, 0)) * M4.rotX(-s * .pi / 2 * 0.85))
        }
        b.path([v(0, 0.4, 0), v(0, 0.35, 0.35), v(0, 0, 0.5), v(0, -0.35, 0.35), v(0, -0.4, 0)], 0.015, segs: 4, part: MID)
    }


    /// Two hoops at right angles about a pip.
    static let marker = make { b in
        b.hoop(1, 0.05, segs: 20, part: LIGHT)
        b.hoop(1, 0.05, segs: 20, part: LIGHT, at: M4.rotX(.pi / 2))
        b.ball(.zero, 0.18, part: LIGHT)
    }
}

extension SIMD4 where Scalar == Float {
    fileprivate var xyz: SIMD3<Float> { SIMD3(x, y, z) }
}
