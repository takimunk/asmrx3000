import Foundation
import simd

/// The things a clip scene's sounds are drawn as, besides the animals and rocks of
/// `ClipModels`: small low-poly models in the stage's grey stone, each about two units across
/// (x forward, y left, z up, the sounding point at the origin) and scaled on stage by
/// `ClipActors`. Light parts are stone, dark ones (iron, windows, silhouettes) wood, the
/// in-between section grey.
enum Prop: CaseIterable {
    // Space
    case freighter, mediumShip, shuttle, convoy, station, fan, pump, panel, rcs, clamp, hatch, robotArm, dish, console, plasma
    // Under water
    case reef, coral, fish, rainSurface
    // Lighthouse
    case clockwork, wind, rainGlass, rain, sea, surf, buoy, foghorn, bolt, pane, stairs
    // Night train
    case track, bogie, crossing, train, bridge, arches, teaGlass, vent, horn, lampPosts
    // Singing ice
    case treeLine, iceStar, zigzag, boom, owl, skater
    // Tea house
    case kettle, brazier, cloth, ladle, whisk, host, teaBowl, eaves, basin, shishi, chime, templeBell
    // Night library (built in metres, drawn at the room's scale)
    case hearth, embers, log, book, inkPen, longcaseClock, bookcase, radiator, boards, door
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
        case .freighter: return freighter
        case .mediumShip: return mediumShip
        case .shuttle: return shuttle
        case .convoy: return convoy
        case .station: return station
        case .fan: return fanFrame
        case .pump: return pump
        case .panel: return panel
        case .rcs: return rcs
        case .clamp: return clamp
        case .hatch: return hatch
        case .robotArm: return robotArm
        case .dish: return dish
        case .console: return console
        case .plasma: return plasma
        case .reef: return reef
        case .coral: return coral
        case .fish: return fish
        case .rainSurface: return droplets
        case .clockwork: return clockFrame
        case .wind: return wind
        case .rainGlass: return rainGlass
        case .rain: return rain
        case .sea: return crest
        case .surf: return surf
        case .buoy: return buoy
        case .foghorn: return foghorn
        case .bolt: return bolt
        case .pane: return pane
        case .stairs: return stairs
        case .track, .bridge: return nil   // built in place (ClipActors.inPlace)
        case .bogie: return bogie
        case .crossing: return crossing
        case .train: return train
        case .arches: return arch
        case .teaGlass: return teaGlass
        case .vent: return vent
        case .horn: return horn
        case .lampPosts: return lampPosts
        case .treeLine: return treeLine
        case .iceStar: return iceStar
        case .zigzag: return zigzag
        case .boom: return hoop
        case .owl: return owl
        case .skater: return skater
        case .kettle: return kettle
        case .brazier: return brazier
        case .cloth: return cloth
        case .ladle: return ladle
        case .whisk: return whisk
        case .host: return host
        case .teaBowl: return teaBowl
        case .eaves: return eaves
        case .basin: return basin
        case .shishi: return shishiFrame
        case .chime: return chime
        case .templeBell: return templeBell
        case .hearth: return hearth
        case .embers: return embers
        case .log: return log
        case .book: return book
        case .inkPen: return inkPen
        case .longcaseClock: return longcaseClock
        case .bookcase: return bookcase
        case .radiator: return radiator
        case .boards: return boards
        case .door: return door
        case .bead: return bead
        case .singingBowl: return singingBowl
        case .tingsha: return tingsha
        case .marker: return marker
        }
    }

    // MARK: Space

    static let freighterParts: MeshBuilder = builder { b in
        b.box(v(0.05, 0, 0), v(1.7, 0.34, 0.24), part: LIGHT)
        for i in 0..<4 { b.box(v(-0.5 + 0.3 * Float(i), 0, 0.19), v(0.26, 0.3, 0.14), part: MID) }
        b.box(v(0.62, 0, 0.2), v(0.16, 0.26, 0.16), part: LIGHT)
        b.box(v(0.66, 0, 0.25), v(0.1, 0.28, 0.04), part: DARK)
        b.box(v(0.95, 0, -0.02), v(0.14, 0.24, 0.16), part: LIGHT)
        b.box(v(-0.86, 0, 0), v(0.18, 0.44, 0.32), part: LIGHT)
        for (y, z) in [(-0.12, 0.07), (0.12, 0.07), (-0.12, -0.08), (0.12, -0.08)] as [(Float, Float)] {
            b.rod(v(-0.95, y, z), v(-1.1, y, z), 0.05, 0.08, segs: 8, part: DARK)
        }
        b.rod(v(0.62, 0, 0.28), v(0.62, 0, 0.5), 0.012, part: LIGHT)
    }

    static let freighter = make { $0 = freighterParts }

    static let mediumShip = make { b in
        b.lathe([(0.95, 0.01, 0.01, 0), (0.8, 0.08, 0.06, 0), (0.4, 0.16, 0.11, 0), (-0.4, 0.17, 0.12, 0), (-0.72, 0.14, 0.1, 0)],
                segs: 8, part: LIGHT)
        b.box(v(-0.72, 0, 0), v(0.04, 0.28, 0.2), part: LIGHT)
        for s: Float in [1, -1] {
            b.sheet([v2(0.25, 0.1), v2(-0.45, 0.68), v2(-0.66, 0.68), v2(-0.55, 0.1)], 0.03, at: v(0, 0, -0.02),
                    u: X, v: Y * s, part: LIGHT)
            b.rod(v(-0.72, 0.08 * s, 0), v(-0.9, 0.08 * s, 0), 0.05, 0.075, segs: 8, part: DARK)
            b.rod(v(-0.5, 0.62 * s, -0.02), v(-0.72, 0.62 * s, -0.02), 0.035, 0.05, segs: 6, part: DARK)
        }
        b.sheet([v2(-0.2, 0.08), v2(-0.55, 0.36), v2(-0.7, 0.36), v2(-0.66, 0.08)], 0.025, at: .zero, u: X, v: Z, part: LIGHT)
        b.box(v(0.55, 0, 0.09), v(0.22, 0.12, 0.05), part: DARK)
    }

    static let shuttle = make { b in
        b.lathe([(0.9, 0.01, 0.01, 0), (0.75, 0.09, 0.08, 0.01), (0.4, 0.15, 0.13, 0.02), (-0.6, 0.15, 0.13, 0.02), (-0.72, 0.13, 0.12, 0.02)],
                segs: 8, part: LIGHT)
        b.box(v(-0.72, 0, 0.02), v(0.04, 0.26, 0.24), part: LIGHT)
        b.sheet([v2(0.3, 0), v2(-0.62, 0.58), v2(-0.74, 0.52), v2(-0.74, -0.52), v2(-0.62, -0.58)], 0.035, at: v(0, 0, -0.06),
                u: X, v: Y, part: LIGHT)
        b.sheet([v2(-0.3, 0.1), v2(-0.62, 0.48), v2(-0.74, 0.48), v2(-0.72, 0.1)], 0.03, at: v(0, 0, 0.02), u: X, v: Z, part: LIGHT)
        b.rod(v(-0.72, 0, 0.02), v(-0.9, 0, 0.02), 0.07, 0.1, segs: 8, part: DARK)
        b.box(v(0.6, 0, 0.11), v(0.16, 0.16, 0.05), part: DARK)
    }

    /// Three freighters in a loose line.
    static let convoy = make { b in
        for (o, s) in [(v(0.6, 0.3, 0.05), Float(0.36)), (v(-0.05, -0.2, 0), 0.32), (v(-0.7, 0.18, -0.08), 0.3)] {
            b.add(freighterParts, M4.move(o) * M4.scale(s))
        }
    }

    /// A station module: a pressurised cylinder with ribs and a docking node, a truss to two
    /// solar wings.
    static let station = make { b in
        b.lathe([(0.75, 0.001, 0.001, 0), (0.72, 0.12, 0.12, 0), (0.64, 0.22, 0.22, 0), (-0.6, 0.22, 0.22, 0),
                 (-0.68, 0.12, 0.12, 0), (-0.7, 0.001, 0.001, 0)], segs: 12, part: LIGHT)
        for x: Float in [-0.35, 0, 0.35] { b.hoop(0.225, 0.02, segs: 16, part: MID, at: M4.move(v(x, 0, 0)) * M4.rotY(.pi / 2)) }
        b.ball(v(0.86, 0, 0), 0.13, part: LIGHT)
        b.rod(v(0.86, 0, 0.1), v(0.86, 0, 0.3), 0.08, part: LIGHT)
        b.hoop(0.08, 0.02, segs: 10, part: DARK, at: M4.move(v(0.86, 0, 0.3)))
        for s: Float in [1, -1] {
            // Truss: two rails and diagonals.
            let y0: Float = 0.22, y1: Float = 0.95
            for z: Float in [-0.05, 0.05] { b.rod(v(0, y0 * s, z), v(0, y1 * s, z), 0.014, segs: 4, part: MID) }
            var y = y0
            var up = true
            while y < y1 - 0.01 {
                let y2 = min(y + 0.1, y1)
                b.rod(v(0, y * s, up ? -0.05 : 0.05), v(0, y2 * s, up ? 0.05 : -0.05), 0.009, segs: 4, part: MID)
                y = y2
                up.toggle()
            }
            b.box(v(0, 1.0 * s, 0), v(0.62, 0.36, 0.014), part: MID)
            for k in -2...2 { b.box(v(0.12 * Float(k), 1.0 * s, 0.009), v(0.012, 0.36, 0.006), part: DARK) }
        }
    }

    static let fanFrame = make { b in
        b.hoop(0.78, 0.08, segs: 24, part: LIGHT, at: M4.rotY(.pi / 2))
        for (c, s) in [(v(0, 0, 0.9), v(0.12, 1.92, 0.12)), (v(0, 0, -0.9), v(0.12, 1.92, 0.12)),
                       (v(0, 0.9, 0), v(0.12, 0.12, 1.92)), (v(0, -0.9, 0), v(0.12, 0.12, 1.92))] {
            b.box(c, s, part: MID)
        }
        for k in -2...2 {
            b.box(v(0.1, 0.3 * Float(k), 0), v(0.02, 0.025, 1.5), part: LIGHT)
        }
        b.ball(v(-0.05, 0, 0), 0.16, part: MID)
    }

    /// Five blades about x.
    static let fanBlades = make { b in
        for k in 0..<5 {
            let a = 2 * Float.pi * Float(k) / 5
            let u = v(0.25, cos(a), sin(a)), w = v(0, -sin(a), cos(a))
            b.sheet([v2(0.1, -0.06), v2(0.68, -0.2), v2(0.72, 0.12), v2(0.12, 0.08)], 0.025, at: v(-0.05, 0, 0),
                    u: simd_normalize(u), v: w, part: LIGHT)
        }
    }

    static let pump = make { b in
        b.box(v(0, 0, -0.62), v(1.6, 0.9, 0.08), part: MID)
        b.rod(v(-0.85, 0, -0.2), v(-0.15, 0, -0.2), 0.26, segs: 12, part: MID)
        for x: Float in [-0.7, -0.55, -0.4, -0.25] { b.hoop(0.27, 0.02, segs: 12, part: DARK, at: M4.move(v(x, 0, -0.2)) * M4.rotY(.pi / 2)) }
        b.rod(v(-0.05, -0.28, -0.2), v(-0.05, 0.28, -0.2), 0.34, segs: 14, part: LIGHT)
        b.rod(v(-0.05, 0, 0.1), v(-0.05, 0, 0.8), 0.1, segs: 8, part: LIGHT)
        b.rod(v(-0.05, 0, 0.55), v(-0.05, 0, 0.6), 0.16, segs: 10, part: MID)
        b.rod(v(0.25, 0, -0.2), v(0.95, 0, -0.2), 0.1, segs: 8, part: LIGHT)
        b.rod(v(0.6, 0, -0.2), v(0.65, 0, -0.2), 0.16, segs: 10, part: MID)
        b.rod(v(-0.85, 0, -0.45), v(-0.85, 0, -0.58), 0.12, part: MID)
    }

    /// A hull plate with ribs and stringers, facing +x.
    static let panel = make { b in
        b.box(v(0, 0, 0), v(0.06, 1.7, 1.1), part: LIGHT)
        for y: Float in [-0.55, 0, 0.55] { b.box(v(-0.07, y, 0), v(0.12, 0.07, 1.1), part: MID) }
        for z: Float in [-0.35, 0.35] { b.box(v(-0.05, 0, z), v(0.08, 1.7, 0.05), part: MID) }
        for y in stride(from: Float(-0.75), through: 0.75, by: 0.25) {
            for z: Float in [-0.5, 0.5] { b.ball(v(0.04, y, z), 0.025, part: DARK) }
        }
    }

    /// A reaction-control quad: a block with four nozzles, on an arm.
    static let rcs = make { b in
        b.box(.zero, v(0.5, 0.5, 0.5), part: LIGHT)
        for d in [Y, -Y, Z, -Z] { b.rod(d * 0.25, d * 0.62, 0.08, 0.18, segs: 8, part: DARK) }
        b.rod(v(-0.25, 0, 0), v(-0.95, 0, 0), 0.07, segs: 6, part: MID)
        b.box(v(-0.95, 0, 0), v(0.1, 0.4, 0.4), part: MID)
    }

    /// Docking clamp: a base and two jaws.
    static let clamp = make { b in
        b.box(v(-0.45, 0, 0), v(0.3, 0.6, 0.7), part: MID)
        for s: Float in [1, -1] {
            b.box(v(0.1, 0, 0.26 * s), v(0.85, 0.32, 0.12), part: LIGHT, rot: M4.rotY(0.28 * s))
            b.box(v(0.52, 0, 0.1 * s), v(0.14, 0.32, 0.26), part: LIGHT)
            b.rod(v(-0.3, -0.2, 0.25 * s), v(-0.3, 0.2, 0.25 * s), 0.06, part: DARK)
        }
    }

    /// An airlock hatch facing +x: frame, door and its wheel (`hatchWheel` turns).
    static let hatch = make { b in
        b.hoop(0.76, 0.1, segs: 24, part: LIGHT, at: M4.rotY(.pi / 2))
        b.rod(v(-0.05, 0, 0), v(0.05, 0, 0), 0.68, segs: 20, part: MID)
        for z: Float in [-0.4, 0.4] { b.box(v(0.0, -0.82, z), v(0.12, 0.16, 0.18), part: DARK) }
        b.box(v(-0.06, 0, 0), v(0.06, 2.0, 2.0), part: DARK)
    }

    static let hatchWheel = make { b in
        b.hoop(0.32, 0.035, segs: 16, part: LIGHT, at: M4.move(v(0.12, 0, 0)) * M4.rotY(.pi / 2))
        for k in 0..<3 {
            let a = Float(k) * .pi / 3
            b.rod(v(0.12, 0.32 * cos(a), 0.32 * sin(a)), v(0.12, -0.32 * cos(a), -0.32 * sin(a)), 0.02, segs: 4, part: LIGHT)
        }
        b.rod(v(0.05, 0, 0), v(0.16, 0, 0), 0.06, part: LIGHT)
    }

    static let robotArm = make { b in
        b.rod(v(-0.4, 0, -0.85), v(-0.4, 0, -0.68), 0.3, segs: 12, part: MID)
        b.rod(v(-0.4, 0, -0.68), v(-0.4, 0, -0.45), 0.16, segs: 10, part: LIGHT)
        b.ball(v(-0.4, 0, -0.42), 0.13, part: MID)
        b.rod(v(-0.4, 0, -0.42), v(0.05, 0, 0.25), 0.085, segs: 8, part: LIGHT)
        b.ball(v(0.05, 0, 0.25), 0.11, part: MID)
        b.rod(v(0.05, 0, 0.25), v(0.7, 0, 0.12), 0.07, segs: 8, part: LIGHT)
        b.ball(v(0.7, 0, 0.12), 0.08, part: MID)
        b.box(v(0.78, 0, 0.12), v(0.1, 0.2, 0.08), part: MID)
        for s: Float in [1, -1] { b.box(v(0.9, 0.08 * s, 0.12), v(0.18, 0.03, 0.06), part: LIGHT) }
    }

    /// A small dish on a mast, looking up and out (+x).
    static let dish = make { b in
        b.box(v(0, 0, -0.85), v(0.5, 0.5, 0.1), part: MID)
        b.rod(v(0, 0, -0.8), v(0, 0, -0.1), 0.05, part: LIGHT)
        let tilt = M4.move(v(0, 0, 0)) * M4.rotY(.pi / 2 - 0.6)
        b.turn([v2(0, -0.02), v2(0.2, 0.0), v2(0.4, 0.06), v2(0.6, 0.15), v2(0.72, 0.22), v2(0.7, 0.24),
                v2(0.58, 0.18), v2(0.38, 0.09), v2(0.18, 0.03), v2(0, 0.0)], segs: 18, part: LIGHT, at: tilt)
        let focus = (tilt * SIMD4(0, 0, 0.55, 1))
        let f = v(focus.x, focus.y, focus.z)
        b.rod(.zero, f, 0.02, segs: 4, part: MID)
        for k in 0..<3 {
            let a = 2 * Float.pi * Float(k) / 3
            let rim = tilt * SIMD4(0.65 * cos(a), 0.65 * sin(a), 0.2, 1)
            b.rod(v(rim.x, rim.y, rim.z), f, 0.012, segs: 4, part: MID)
        }
        b.ball(f, 0.06, part: DARK)
    }

    /// A radio console facing +x: desk, sloped panel, screen, keys and a whip aerial.
    static let console = make { b in
        b.box(v(0, 0, -0.45), v(0.7, 1.4, 0.7), part: LIGHT)
        b.box(v(0.05, 0, -0.06), v(0.6, 1.4, 0.06), part: LIGHT, rot: M4.rotY(0.35))
        b.box(v(-0.28, 0, 0.32), v(0.08, 1.1, 0.6), part: MID, rot: M4.rotY(-0.12))
        b.box(v(-0.22, 0, 0.33), v(0.02, 0.9, 0.45), part: DARK, rot: M4.rotY(-0.12))
        for i in 0..<3 {
            for j in 0..<4 {
                b.box(v(0.08 + 0.13 * Float(i), -0.39 + 0.26 * Float(j), -0.03 - 0.05 * Float(i)), v(0.07, 0.12, 0.05), part: DARK, rot: M4.rotY(0.35))
            }
        }
        b.rod(v(-0.3, 0.55, 0.6), v(-0.36, 0.62, 1.05), 0.015, segs: 4, part: LIGHT)
        b.ball(v(-0.36, 0.62, 1.05), 0.04, part: LIGHT)
    }

    /// Field lines of a dipole: eight loops in four planes about z.
    static let plasma = make { b in
        for m in 0..<4 {
            let phi = Float(m) * .pi / 4 + 0.2
            for L: Float in [1.0, 0.62] {
                var pts: [SIMD3<Float>] = []
                for i in 0...16 {
                    let th = 0.28 + (Float.pi - 0.56) * Float(i) / 16
                    let r = L * sin(th) * sin(th)
                    pts.append(v(r * sin(th) * cos(phi), r * sin(th) * sin(phi), r * cos(th) * 0.8))
                }
                b.path(pts, 0.01, segs: 4, part: LIGHT)
            }
        }
    }

    // MARK: Under water

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

    // MARK: Lighthouse

    /// A Fresnel lens: a ribbed drum with bull's-eye panels on four sides, on a pedestal.
    static let lens = make { $0 = lensParts }

    /// The lighthouse's beam: a long cone along +x from the lens, drawn see-through.
    static let lightBeam = make { b in
        b.rod(v(0.3, 0, 0), v(1, 0, 0), 0.03, 0.16, segs: 12, part: LIGHT)
    }

    static let lensParts: MeshBuilder = builder { b in
        var prof: [SIMD2<Float>] = [v2(0, -0.62), v2(0.5, -0.62)]
        for i in 0..<12 {
            let z = -0.6 + 0.1 * Float(i)
            prof.append(v2(0.5, z))
            prof.append(v2(0.56, z + 0.06))
        }
        prof += [v2(0.5, 0.6), v2(0.42, 0.72), v2(0.2, 0.86), v2(0.0, 0.9)]
        b.turn(prof, segs: 16, part: LIGHT)
        for k in 0..<4 {
            let rot = M4.rotZ(Float(k) * .pi / 2)
            for (R, z) in [(Float(0.1), Float(0)), (0.2, 0), (0.3, 0)] {
                b.hoop(R, 0.025, segs: 14, part: MID, at: rot * M4.move(v(0.58, 0, z)) * M4.rotY(.pi / 2))
            }
            b.ball((rot * SIMD4(0.58, 0, 0, 1)).xyz, 0.06, part: MID)
        }
        b.rod(v(0, 0, -0.62), v(0, 0, -1.0), 0.22, 0.3, segs: 10, part: MID)
    }

    static let clockFrame = make { b in
        for y: Float in [-0.25, 0.25] { b.box(v(0, y, -0.1), v(0.9, 0.05, 0.9), part: MID) }
        b.rod(v(0, -0.35, 0.1), v(0, 0.35, 0.1), 0.04, part: LIGHT)
        b.rod(v(0.25, -0.3, -0.25), v(0.25, 0.3, -0.25), 0.07, segs: 10, part: LIGHT)
        b.rod(v(0.25, 0.3, -0.25), v(0.25, 0.3, -0.95), 0.012, segs: 4, part: LIGHT)
        b.box(v(0.25, 0.3, -1.0), v(0.14, 0.14, 0.2), part: DARK)
        b.box(v(0, 0, -0.6), v(1.0, 0.7, 0.06), part: MID)
    }

    /// A toothed wheel about y (centre at (0, 0, 0.1) of the clockwork).
    static let gear = make { b in
        var poly: [SIMD2<Float>] = []
        let n = 16
        for i in 0..<(n * 2) {
            let a = Float.pi * Float(i) / Float(n)
            let r: Float = i % 2 == 0 ? 0.42 : 0.35
            poly.append(v2(r * cos(a - 0.06), r * sin(a - 0.06)))
            poly.append(v2(r * cos(a + 0.06), r * sin(a + 0.06)))
        }
        b.sheet(poly, 0.06, at: .zero, u: X, v: Z, part: LIGHT)
        b.hoop(0.2, 0.025, segs: 14, part: MID, at: M4.rotX(.pi / 2))
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

    /// Rain running down a pane, facing +x: only the streaks and their drops (the glass is the
    /// place's own: the lantern's glazing, the compartment's window).
    static let rainGlass = make { b in
        var rng = SeededGenerator(3)
        for k in 0..<7 {
            let y = -0.45 + 0.15 * Float(k) + 0.05 * rng.next()
            let top = 0.6 - 0.5 * rng.next()
            var pts: [SIMD3<Float>] = []
            for i in 0...6 { pts.append(v(0.03, y + 0.03 * sin(Float(i) * 1.7 + Float(k)), top - (top + 0.65) * Float(i) / 6)) }
            b.path(pts, 0.012, 0.02, segs: 4, part: LIGHT)
            b.ball(pts[6] + v(0, 0, 0.02), 0.035, part: LIGHT)
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

    /// A breaking wave: a curling sheet and a burst of spray.
    static let surf = make { b in
        let curve: [SIMD2<Float>] = [v2(-0.6, 0), v2(-0.3, 0.08), v2(-0.05, 0.28), v2(0.1, 0.5), v2(0.25, 0.56), v2(0.38, 0.46), v2(0.4, 0.3)]
        var tris: [(SIMD3<Float>, SIMD3<Float>, SIMD3<Float>)] = []
        let xs: [Float] = [-1, -0.6, -0.2, 0.2, 0.6, 1]
        func pt(_ x: Float, _ c: SIMD2<Float>) -> SIMD3<Float> { v(x, c.x, c.y * (1 - 0.7 * x * x)) }
        for i in 0..<(xs.count - 1) {
            for j in 0..<(curve.count - 1) {
                let a = pt(xs[i], curve[j]), b2 = pt(xs[i + 1], curve[j]), c = pt(xs[i + 1], curve[j + 1]), d = pt(xs[i], curve[j + 1])
                tris.append((a, b2, c))
                tris.append((a, c, d))
            }
        }
        b.flat(tris, part: Part.water)
        var rng = SeededGenerator(8)
        for _ in 0..<10 {
            let x = -0.6 + 1.2 * rng.next()
            b.ball(v(x, 0.3 + 0.2 * rng.next(), (0.55 + 0.25 * rng.next()) * (1 - 0.7 * x * x)), 0.04 + 0.03 * rng.next(), part: LIGHT)
        }
    }

    /// A bell buoy: float, cage, bell, lamp.
    static let buoy = make { b in
        b.turn([v2(0, -0.5), v2(0.3, -0.46), v2(0.42, -0.25), v2(0.44, -0.05), v2(0.36, 0.05), v2(0, 0.06)], segs: 14, part: MID)
        b.hoop(0.44, 0.03, segs: 16, part: LIGHT, at: M4.move(v(0, 0, -0.1)))
        for k in 0..<4 {
            let a = Float(k) * .pi / 2 + .pi / 4
            b.rod(v(0.3 * cos(a), 0.3 * sin(a), 0.05), v(0.1 * cos(a), 0.1 * sin(a), 0.85), 0.025, segs: 4, part: LIGHT)
        }
        b.hoop(0.18, 0.02, segs: 12, part: LIGHT, at: M4.move(v(0, 0, 0.62)))
        b.turn([v2(0, 0.62), v2(0.07, 0.6), v2(0.11, 0.52), v2(0.13, 0.4), v2(0.18, 0.33), v2(0, 0.33)], segs: 12, part: LIGHT)
        b.ball(v(0, 0, 0.9), 0.07, part: LIGHT)
    }

    /// A foghorn: a squat house with a trumpet on its roof facing +x.
    static let foghorn = make { b in
        b.box(v(-0.2, 0, -0.45), v(0.8, 0.7, 0.5), part: MID)
        for s: Float in [1, -1] { b.box(v(-0.2, 0.18 * s, -0.08), v(0.86, 0.44, 0.05), part: DARK, rot: M4.rotX(-0.55 * s)) }
        b.box(v(0.21, 0.1, -0.5), v(0.02, 0.14, 0.24), part: DARK)
        b.turn([v2(0.06, -0.4), v2(0.07, 0.15), v2(0.12, 0.38), v2(0.22, 0.52), v2(0.32, 0.58), v2(0.3, 0.6), v2(0.2, 0.55)],
               segs: 14, part: LIGHT, at: M4.move(v(0.0, 0, 0.18)) * M4.rotY(.pi / 2))
        b.rod(v(-0.3, 0, 0.0), v(-0.3, 0, 0.18), 0.06, part: MID)
    }

    /// A lightning glyph.
    static let bolt = make { b in
        b.path([v(-0.1, 0, 1.0), v(0.15, 0, 0.25), v(-0.15, 0, 0.12), v(0.08, 0, -0.95)], 0.07, 0.025, segs: 5, part: LIGHT)
        b.rod(v(0.15, 0, 0.25), v(0.4, 0, -0.1), 0.035, 0.012, segs: 4, part: LIGHT)
    }

    /// A rattling pane of the lantern's glazing, facing +x: its frame alone, lit up on the glass
    /// where it sounds (the glazing itself is the place's).
    static let pane = make { b in
        for (c, s) in [(v(0, 0, 0.8), v(0.06, 1.5, 0.06)), (v(0, 0, -0.8), v(0.06, 1.5, 0.06)),
                       (v(0, 0.72, 0), v(0.06, 0.06, 1.66)), (v(0, -0.72, 0), v(0.06, 0.06, 1.66))] {
            b.box(c, s, part: LIGHT)
        }
    }

    /// A turn of the spiral stair with its handrail.
    static let stairs = make { b in
        b.rod(v(0, 0, -1), v(0, 0, 1), 0.07, segs: 8, part: MID)
        var rail: [SIMD3<Float>] = []
        for k in 0..<8 {
            let a = Float(k) * 0.62
            let z = -0.9 + 0.24 * Float(k)
            b.box(v(0.36 * cos(a), 0.36 * sin(a), z), v(0.62, 0.24, 0.05), part: LIGHT, rot: M4.rotZ(a))
            let post = v(0.64 * cos(a), 0.64 * sin(a), z)
            b.rod(post, post + v(0, 0, 0.4), 0.014, segs: 4, part: LIGHT)
            rail.append(post + v(0, 0, 0.4))
        }
        b.path(rail, 0.025, segs: 5, part: LIGHT)
    }

    // MARK: Night train

    /// A two-axle bogie along x.
    static let bogie = make { b in
        for s: Float in [1, -1] {
            b.box(v(0, 0.42 * s, 0.05), v(1.5, 0.08, 0.2), part: MID)
            for x: Float in [-0.45, 0.45] {
                b.rod(v(x, 0.36 * s, 0), v(x, 0.48 * s, 0), 0.3, segs: 14, part: LIGHT)
                b.rod(v(x, 0.48 * s, 0), v(x, 0.52 * s, 0), 0.12, segs: 8, part: DARK)
            }
            b.rod(v(0, 0.42 * s, 0.15), v(0, 0.42 * s, 0.4), 0.09, segs: 8, part: DARK)
        }
        for x: Float in [-0.45, 0.45] { b.rod(v(x, -0.4, 0), v(x, 0.4, 0), 0.05, part: MID) }
        b.box(v(0, 0, 0.15), v(0.2, 0.9, 0.12), part: MID)
        b.box(v(0, 0, -0.33), v(1.9, 0.05, 0.04), part: DARK)
    }

    /// A level-crossing post facing +x: crossbuck, two lamps, the bell on top.
    static let crossing = make { b in
        b.rod(v(0, 0, -1.0), v(0, 0, 0.75), 0.045, segs: 6, part: LIGHT)
        b.box(v(0, 0, -1.0), v(0.3, 0.3, 0.1), part: MID)
        for s: Float in [1, -1] {
            b.box(v(0.06, 0, 0.42), v(0.03, 0.9, 0.12), part: LIGHT, rot: M4.rotX(0.6 * s))
            b.rod(v(0.03, 0.3 * s, 0.0), v(0.18, 0.3 * s, 0.0), 0.1, 0.12, segs: 10, part: DARK)
        }
        b.box(v(0, 0, 0.0), v(0.06, 0.7, 0.06), part: MID)
        b.turn([v2(0, 0.95), v2(0.08, 0.94), v2(0.13, 0.86), v2(0.14, 0.76), v2(0, 0.76)], segs: 12, part: LIGHT)
    }

    /// A passing train: a locomotive and three cars, ~4 long along x.
    static let train = make { b in
        b.box(v(1.45, 0, 0.0), v(0.9, 0.34, 0.4), part: LIGHT)
        b.box(v(1.95, 0, -0.04), v(0.14, 0.34, 0.32), part: LIGHT, rot: M4.rotY(-0.25))
        b.box(v(1.82, 0, 0.12), v(0.04, 0.3, 0.1), part: DARK, rot: M4.rotY(-0.25))
        for i in 0..<3 {
            let x = 0.45 - 1.03 * Float(i)
            b.box(v(x, 0, 0), v(0.98, 0.34, 0.38), part: LIGHT)
            b.box(v(x, 0, 0.07), v(0.86, 0.35, 0.07), part: DARK)
            b.box(v(x, 0, 0.21), v(0.94, 0.26, 0.04), part: MID)
        }
        for x: Float in [1.7, 1.2, 0.8, 0.1, -0.25, -0.95, -1.3, -2.0] { b.box(v(x, 0, -0.23), v(0.22, 0.26, 0.08), part: DARK) }
    }

    /// A tunnel arch: an open ring in the y-z plane of radius 1 (the floor gap down).
    static let arch = make { b in
        b.hoop(1, 0.03, segs: 26, from: -0.45, to: .pi + 0.45, part: MID, at: M4.rotY(.pi / 2) * M4.rotZ(.pi / 2))
    }

    /// Tea in a glass holder on a little table: the glass at the origin.
    static let teaGlass = make { b in
        b.box(v(0, 0, -0.42), v(1.1, 0.8, 0.05), part: MID)
        b.rod(v(0, 0, -0.45), v(0, 0, -1.2), 0.05, part: DARK)
        b.box(v(0, 0, -1.2), v(0.5, 0.4, 0.04), part: DARK)
        b.turn([v2(0, -0.39), v2(0.2, -0.39), v2(0.19, -0.33), v2(0.2, -0.05), v2(0.22, 0.0), v2(0.2, 0.0), v2(0.17, -0.3), v2(0, -0.3)],
               segs: 14, part: LIGHT)
        b.turn([v2(0.17, -0.02), v2(0.19, 0.36), v2(0.17, 0.36), v2(0.15, -0.02)], segs: 14, part: Part.water)
        b.hoop(0.12, 0.022, segs: 10, from: -1.4, to: 1.4, part: LIGHT, at: M4.move(v(0, -0.22, -0.15)) * M4.rotX(.pi / 2) * M4.rotZ(-.pi / 2))
        b.rod(v(0.02, 0.05, -0.1), v(0.1, 0.12, 0.55), 0.012, segs: 4, part: LIGHT)
    }

    /// A louvred vent facing +x.
    static let vent = make { b in
        for (c, s) in [(v(0, 0, 0.6), v(0.1, 1.3, 0.1)), (v(0, 0, -0.6), v(0.1, 1.3, 0.1)),
                       (v(0, 0.6, 0), v(0.1, 0.1, 1.3)), (v(0, -0.6, 0), v(0.1, 0.1, 1.3))] {
            b.box(c, s, part: LIGHT)
        }
        for k in 0..<6 { b.box(v(0, 0, -0.45 + 0.18 * Float(k)), v(0.16, 1.1, 0.025), part: MID, rot: M4.rotY(0.6)) }
        b.box(v(-0.08, 0, 0), v(0.02, 1.2, 1.2), part: DARK)
    }

    /// Twin air horns on a bracket, bells facing +x.
    static let horn = make { b in
        b.box(v(-0.3, 0, -0.3), v(0.5, 0.6, 0.1), part: MID)
        for (y, l) in [(Float(0.14), Float(0.9)), (-0.14, 0.7)] {
            b.turn([v2(0.05, 0), v2(0.06, l * 0.6), v2(0.1, l * 0.85), v2(0.2, l), v2(0.18, l)], segs: 12, part: LIGHT,
                   at: M4.move(v(-0.45, y, -0.1)) * M4.rotY(.pi / 2))
            b.rod(v(-0.35, y, -0.25), v(-0.35, y, -0.1), 0.03, part: MID)
        }
    }

    /// A platform edge with three lamp posts, lamps leaning to -y (the track).
    static let lampPosts = make { b in
        b.box(v(0, 0.2, -0.75), v(2.4, 0.6, 0.1), part: MID)
        b.box(v(0, -0.08, -0.72), v(2.4, 0.06, 0.06), part: LIGHT)
        for x: Float in [-0.85, 0, 0.85] {
            b.rod(v(x, 0.3, -0.7), v(x, 0.3, 0.55), 0.03, segs: 6, part: LIGHT)
            b.rod(v(x, 0.3, 0.55), v(x, 0.05, 0.6), 0.02, segs: 4, part: LIGHT)
            b.rod(v(x, 0.05, 0.62), v(x, 0.05, 0.48), 0.03, 0.11, segs: 10, part: MID)
            b.ball(v(x, 0.05, 0.47), 0.05, part: LIGHT)
        }
        b.box(v(0, 0.3, 0.3), v(0.5, 0.03, 0.16), part: LIGHT)
    }

    // MARK: Singing ice

    /// A shore of firs along y, facing +x.
    static let treeLine = make { b in
        b.box(v(0, 0, -0.18), v(0.5, 2.6, 0.08), part: MID)
        var rng = SeededGenerator(31)
        for i in 0..<12 {
            let y = -1.2 + 0.22 * Float(i) + 0.06 * rng.next()
            let h = 0.35 + 0.5 * rng.next()
            let x = -0.15 + 0.3 * rng.next()
            b.rod(v(x, y, -0.15), v(x, y, -0.15 + h * 0.6), 0.1 * h, 0.0, segs: 6, part: DARK)
            b.rod(v(x, y, -0.15 + h * 0.3), v(x, y, -0.15 + h), 0.075 * h, 0.0, segs: 6, part: DARK)
        }
    }

    /// A crack starburst lying on the ice, with a shard at its heart.
    static let iceStar = make { b in
        var rng = SeededGenerator(23)
        for k in 0..<7 {
            let a = Float(k) * 0.9 + 0.3 * rng.next()
            let l = 0.6 + 0.4 * rng.next()
            var pts: [SIMD3<Float>] = [.zero]
            for i in 1...4 {
                let r = l * Float(i) / 4
                let aa = a + 0.18 * (rng.next() - 0.5)
                pts.append(v(r * cos(aa), r * sin(aa), 0))
            }
            b.path(pts, 0.035, 0.012, segs: 4, part: LIGHT)
            let m = pts[2]
            b.rod(m, m + v(0.3 * cos(a + 0.7), 0.3 * sin(a + 0.7), 0) * l, 0.02, 0.008, segs: 4, part: LIGHT)
        }
        b.rod(v(0, 0, -0.02), v(0.02, 0.01, 0.28), 0.08, 0.0, segs: 4, part: LIGHT)
    }

    /// A crack running along -x behind its head at the origin.
    static let zigzag = make { b in
        var pts: [SIMD3<Float>] = []
        var rng = SeededGenerator(29)
        for i in 0...10 {
            let x = -2 * Float(i) / 10
            pts.append(v(x, (i % 2 == 0 ? 1 : -1) * (0.05 + 0.1 * rng.next()), 0))
        }
        b.path(pts, 0.05, 0.012, segs: 4, part: LIGHT)
        for i in [2, 5, 7] {
            b.rod(pts[i], pts[i] + v(-0.25, pts[i].y > 0 ? 0.25 : -0.25, 0), 0.025, 0.008, segs: 4, part: LIGHT)
        }
        b.rod(v(0, 0, -0.02), v(0, 0, 0.18), 0.07, 0.0, segs: 4, part: LIGHT)
    }

    /// A flat hoop of radius 1 (the booms' rings).
    static let hoop = make { $0.hoop(1, 0.035, segs: 32, part: LIGHT) }

    /// An owl on a branch, facing +x.
    static let owl = make { b in
        b.rod(v(-0.2, -0.7, -0.5), v(0.1, 0.8, -0.45), 0.05, 0.03, segs: 6, part: DARK)
        b.turn([v2(0, -0.48), v2(0.17, -0.44), v2(0.24, -0.25), v2(0.24, 0.0), v2(0.18, 0.12), v2(0, 0.14)], segs: 12, part: Part.feathers,
               at: M4.scale(v(0.9, 1, 1)))
        b.ball(v(0.02, 0, 0.27), 0.19, part: Part.feathers, squash: v(0.9, 1.05, 0.85))
        for s: Float in [1, -1] {
            b.rod(v(0, 0.1 * s, 0.38), v(-0.02, 0.15 * s, 0.52), 0.05, 0.0, segs: 4, part: Part.feathers)
            b.rod(v(0.14, 0.075 * s, 0.29), v(0.17, 0.075 * s, 0.29), 0.06, segs: 10, part: LIGHT)
            b.ball(v(0.18, 0.075 * s, 0.29), 0.025, part: DARK)
            b.rod(v(0.05, 0.08 * s, -0.47), v(0.12, 0.08 * s, -0.5), 0.02, segs: 4, part: DARK)
        }
        b.rod(v(0.17, 0, 0.24), v(0.24, 0, 0.18), 0.03, 0.0, segs: 4, part: DARK)
        b.turn([v2(0, 0.2), v2(0.2, 0.15)], segs: 10, part: Part.feathers, at: M4.move(v(0, 0, -0.08)))
    }

    /// A skater gliding to +x, leaning in: feet at z 0.
    static let skater = make { b in
        let hip = v(0, 0, 0.62)
        b.rod(hip, v(0.22, 0, 1.05), 0.1, 0.12, segs: 8, part: MID)
        b.ball(v(0.3, 0, 1.18), 0.09, part: LIGHT)
        b.ball(v(0.12, 0, 0.62), 0.11, part: MID)
        for s: Float in [1, -1] {
            b.path([v(0.2, 0.11 * s, 1.0), v(0.0, 0.14 * s, 0.82), v(-0.12, 0.05 * s, 0.7)], 0.045, 0.035, segs: 5, part: MID)
        }
        // Front leg bent under, back leg pushing out behind.
        b.path([v(0.02, 0.07, 0.6), v(0.2, 0.08, 0.32), v(0.1, 0.08, 0.05)], 0.06, 0.045, segs: 6, part: MID)
        b.path([v(-0.02, -0.07, 0.6), v(-0.25, -0.15, 0.32), v(-0.5, -0.22, 0.12)], 0.06, 0.045, segs: 6, part: MID)
        b.box(v(0.12, 0.08, 0.0), v(0.36, 0.02, 0.05), part: LIGHT)
        b.box(v(-0.5, -0.22, 0.06), v(0.36, 0.02, 0.05), part: LIGHT, rot: M4.rotY(-0.3))
    }

    // MARK: Tea house

    /// A cast-iron kettle (tetsubin): squat body, lid knob, spout to +x, a bail handle.
    static let kettle = make { b in
        b.turn([v2(0, -0.3), v2(0.3, -0.28), v2(0.42, -0.12), v2(0.42, 0.08), v2(0.3, 0.2), v2(0.16, 0.24), v2(0, 0.25)], segs: 16, part: MID)
        b.turn([v2(0.17, 0.24), v2(0.17, 0.28), v2(0.0, 0.29)], segs: 12, part: DARK)
        b.ball(v(0, 0, 0.33), 0.055, part: DARK)
        b.rod(v(0.34, 0, -0.02), v(0.62, 0, 0.16), 0.06, 0.035, segs: 6, part: MID)
        b.hoop(0.36, 0.025, segs: 14, from: 0.25, to: .pi - 0.25, part: DARK, at: M4.move(v(0, 0, 0.12)) * M4.rotX(.pi / 2) * M4.rotZ(0))
    }

    /// A brazier (furo) with a few coals.
    static let brazier = make { b in
        b.turn([v2(0, -0.45), v2(0.3, -0.45), v2(0.42, -0.25), v2(0.46, 0.05), v2(0.42, 0.12), v2(0.37, 0.12), v2(0.36, -0.05), v2(0, -0.08)],
               segs: 16, part: LIGHT)
        for k in 0..<3 {
            let a = Float(k) * 2.094
            b.rod(v(0.3 * cos(a), 0.3 * sin(a), -0.4), v(0.34 * cos(a), 0.34 * sin(a), -0.6), 0.05, segs: 5, part: DARK)
        }
        for (x, y) in [(0.1, 0.05), (-0.12, 0.1), (0.0, -0.14), (0.15, -0.1), (-0.1, -0.08)] as [(Float, Float)] {
            b.ball(v(x, y, 0.0), 0.08, part: DARK, squash: v(1.2, 1, 0.8))
        }
    }

    /// A folded silk cloth (fukusa).
    static let cloth = make { b in
        for k in 0..<3 {
            b.box(v(0.02 * Float(k), 0, -0.1 + 0.07 * Float(k)), v(0.9 - 0.04 * Float(k), 0.55, 0.05), part: LIGHT, rot: M4.rotZ(0.04 * Float(k)))
        }
        b.rod(v(0.44, -0.27, 0.0), v(0.44, 0.27, 0.0), 0.06, segs: 8, part: LIGHT)
    }

    /// A bamboo ladle (hishaku): a little cup and a long handle to -x.
    static let ladle = make { b in
        b.turn([v2(0, -0.12), v2(0.14, -0.12), v2(0.14, 0.1), v2(0.12, 0.1), v2(0.12, -0.09), v2(0, -0.09)], segs: 12, part: LIGHT)
        b.rod(v(-0.1, 0, 0.0), v(-1.2, 0, 0.18), 0.03, 0.025, segs: 5, part: LIGHT)
        for x: Float in [-0.45, -0.85] { b.hoop(0.033, 0.012, segs: 8, part: MID, at: M4.move(v(x, 0, 0.066 * (-x - 0.1) / 0.4)) * M4.rotY(.pi / 2 - 0.16)) }
    }

    /// A bamboo whisk (chasen): handle up, tines curling out below.
    static let whisk = make { b in
        b.rod(v(0, 0, 0.05), v(0, 0, 0.75), 0.11, 0.1, segs: 10, part: LIGHT)
        for k in 0..<16 {
            let a = 2 * Float.pi * Float(k) / 16
            let c = cos(a), s = sin(a)
            b.path([v(0.09 * c, 0.09 * s, 0.05), v(0.24 * c, 0.24 * s, -0.25), v(0.27 * c, 0.27 * s, -0.5), v(0.16 * c, 0.16 * s, -0.68)],
                   0.022, 0.014, segs: 3, part: LIGHT)
        }
        b.rod(v(0, 0, 0.05), v(0, 0, -0.4), 0.05, segs: 6, part: MID)
    }

    /// The host kneeling (seiza), facing +x.
    static let host = make { b in
        b.add(Figure.body(kneeling: true), M4.move(v(0, 0, 0.35)) * M4.scale(1.5))
    }

    /// A tea bowl (chawan) with its foot.
    static let teaBowl = make { b in
        b.turn([v2(0, -0.4), v2(0.22, -0.4), v2(0.22, -0.32), v2(0.36, -0.25), v2(0.5, 0.05), v2(0.54, 0.35), v2(0.49, 0.35),
                v2(0.45, 0.06), v2(0.3, -0.16), v2(0, -0.2)], segs: 16, part: LIGHT)
        b.turn([v2(0.44, 0.0), v2(0.0, 0.0)], segs: 12, part: Part.water)
    }

    /// Eaves: the edge of a tiled roof sloping down to +x, drops hanging on it.
    static let eaves = make { b in
        b.box(v(0, 0, 0.2), v(1.0, 2.0, 0.06), part: MID, rot: M4.rotY(0.35))
        for k in 0..<9 {
            let y = -0.9 + 0.225 * Float(k)
            b.rod(v(-0.45, y, 0.4), v(0.47, y, 0.06), 0.045, segs: 6, part: LIGHT)
            b.ball(v(0.5, y, -0.02), 0.04, part: LIGHT, squash: v(0.8, 0.8, 1.5))
        }
        b.box(v(0.47, 0, 0.03), v(0.08, 2.02, 0.08), part: DARK)
    }

    /// A stone wash basin (tsukubai) with its bamboo spout.
    static let basin = make { b in
        b.rock(seed: 61, bumps: 0.15, squash: v(0.55, 0.55, 0.38), part: MID)
        b.turn([v2(0.0, 0.3), v2(0.3, 0.3)], segs: 14, part: Part.water)
        b.rod(v(-1.0, 0, 0.75), v(-0.15, 0, 0.5), 0.05, segs: 6, part: LIGHT)
        b.rod(v(-0.75, 0, -0.35), v(-0.75, 0, 0.68), 0.05, segs: 6, part: LIGHT)
        b.rod(v(0.1, -0.3, 0.42), v(0.1, 0.35, 0.42), 0.02, segs: 4, part: LIGHT)
        for (x, y) in [(0.6, 0.4), (-0.5, -0.55), (0.55, -0.5)] as [(Float, Float)] {
            b.ball(v(x, y, -0.3), 0.14, part: MID, squash: v(1.2, 1, 0.6))
        }
    }

    /// The shishi-odoshi's posts and stone; `shishiTube` rocks on the pivot at (0, 0, 0).
    static let shishiFrame = make { b in
        for s: Float in [1, -1] { b.rod(v(0, 0.13 * s, -0.75), v(0, 0.13 * s, 0.08), 0.04, segs: 6, part: LIGHT) }
        b.rod(v(0, -0.17, 0), v(0, 0.17, 0), 0.018, segs: 4, part: DARK)
        b.ball(v(-0.55, 0, -0.65), 0.18, part: MID, squash: v(1.3, 1, 0.6))
        b.rod(v(0.95, 0, 0.45), v(0.5, 0, 0.35), 0.04, segs: 6, part: LIGHT)
        b.rod(v(0.95, 0, -0.75), v(0.95, 0, 0.47), 0.04, segs: 6, part: LIGHT)
    }

    static let shishiTube = make { b in
        b.rod(v(-0.6, 0, 0), v(0.55, 0, 0), 0.08, segs: 8, part: LIGHT)
        b.hoop(0.08, 0.015, segs: 8, part: MID, at: M4.move(v(0.1, 0, 0)) * M4.rotY(.pi / 2))
    }

    /// A wind chime (furin): bell, clapper, paper strip; it hangs from the origin.
    static let chime = make { b in
        b.rod(v(0, 0, 0.0), v(0, 0, 0.18), 0.012, segs: 4, part: LIGHT)
        b.turn([v2(0, 0.0), v2(0.12, -0.02), v2(0.2, -0.12), v2(0.24, -0.3), v2(0.26, -0.36), v2(0.22, -0.36), v2(0.2, -0.3),
                v2(0.17, -0.14)], segs: 14, part: LIGHT)
        b.rod(v(0, 0, -0.05), v(0, 0, -0.55), 0.01, segs: 4, part: LIGHT)
        b.ball(v(0, 0, -0.3), 0.04, part: DARK)
        b.box(v(0, 0, -0.78), v(0.015, 0.16, 0.46), part: LIGHT)
    }

    /// A temple bell (bonsho) under a small roof, with its log striker.
    static let templeBell = make { b in
        b.turn([v2(0, 0.55), v2(0.3, 0.53), v2(0.4, 0.38), v2(0.42, -0.25), v2(0.5, -0.42), v2(0.46, -0.44), v2(0.38, -0.3), v2(0.0, -0.3)],
               segs: 16, part: MID)
        b.hoop(0.42, 0.025, segs: 16, part: LIGHT, at: M4.move(v(0, 0, -0.05)))
        for (x, y) in [(0.7, 0.62), (-0.7, 0.62), (0.7, -0.62), (-0.7, -0.62)] as [(Float, Float)] {
            b.rod(v(x, y, -0.9), v(x, y, 0.8), 0.05, segs: 6, part: DARK)
        }
        for s: Float in [1, -1] { b.box(v(0.32 * s, 0, 0.98), v(0.95, 1.7, 0.06), part: DARK, rot: M4.rotY(0.5 * s)) }
        b.rod(v(0, 0, 0.55), v(0, 0, 0.8), 0.04, part: DARK)
        b.rod(v(-1.05, 0, -0.1), v(-0.55, 0, -0.1), 0.07, segs: 8, part: LIGHT)
        for x: Float in [-0.95, -0.65] { b.rod(v(x, 0, -0.1), v(x, 0, 0.8), 0.008, segs: 3, part: LIGHT) }
    }

    // MARK: Night library (metres, the sounding point at the origin)

    /// Two andirons in the fireplace with three logs across them; the hearth floor 0.35 below.
    static let hearth = make { b in
        for y: Float in [-0.24, 0.24] {
            b.path([v(-0.22, y, -0.35), v(-0.22, y, -0.12), v(0.16, y, -0.12), v(0.16, y, -0.35)], 0.014, segs: 4, part: DARK)
            b.rod(v(0.16, y, -0.12), v(0.18, y, 0.08), 0.016, segs: 6, part: DARK)
            b.ball(v(0.18, y, 0.1), 0.026, part: DARK)
        }
        b.rod(v(-0.04, -0.33, -0.06), v(-0.04, 0.33, -0.06), 0.065, 0.06, segs: 9, part: MID)
        b.rod(v(0.08, -0.3, -0.07), v(0.06, 0.3, -0.07), 0.055, segs: 9, part: MID)
        b.rod(v(0.0, -0.28, 0.04), v(0.02, 0.26, 0.05), 0.05, 0.045, segs: 9, part: MID)
        for y: Float in [-0.32, 0.32] { b.box(v(-0.03, y, -0.355), v(0.5, 0.06, 0.012), part: LIGHT) }
    }

    /// A bed of coals under the logs.
    static let embers = make { b in
        var rng = SeededGenerator(77)
        for _ in 0..<16 {
            let a = 2 * Float.pi * rng.next(), r = 0.2 * rng.next().squareRoot()
            b.ball(v(r * cos(a), 0.9 * r * sin(a), -0.02 * rng.next()), 0.03 + 0.025 * rng.next(), part: rng.next() > 0.5 ? LIGHT : MID,
                   squash: v(1.2, 1, 0.65))
        }
        b.box(v(0, 0, -0.045), v(0.5, 0.42, 0.012), part: DARK)
    }

    /// A charred log slumping, one end down.
    static let log = make { b in
        b.rod(v(-0.04, -0.24, -0.06), v(0.05, 0.24, 0.06), 0.055, 0.05, segs: 9, part: MID)
        for t: Float in [-0.12, 0.05, 0.18] { b.hoop(0.057, 0.008, segs: 10, part: DARK, at: M4.move(v(0.0, t, t * 0.25)) * M4.rotX(.pi / 2)) }
    }

    /// An open book on the lap, the reader's side to +x, the spine along x; `bookPage` turns over it.
    static let book = make { b in
        for s: Float in [1, -1] {
            b.box(v(0, 0.08 * s, -0.012), v(0.25, 0.16, 0.006), part: DARK, rot: M4.rotX(-0.08 * s))
            b.box(v(0, 0.075 * s, 0.006), v(0.23, 0.145, 0.026), part: LIGHT, rot: M4.rotX(-0.1 * s))
            for k in 1...4 { b.box(v(0.085 - 0.04 * Float(k), 0.075 * s, 0.0205), v(0.004, 0.11, 0.002), part: MID, rot: M4.rotX(-0.1 * s)) }
        }
        b.rod(v(-0.125, 0, -0.008), v(0.125, 0, -0.008), 0.012, segs: 6, part: DARK)
    }

    /// One leaf, hinged on the spine (the x axis), lying to +y.
    static let bookPage = make { b in
        b.box(v(0, 0.072, 0.0), v(0.22, 0.14, 0.002), part: LIGHT)
    }

    /// A sheet of paper, an inkwell and a dip pen on the writing table.
    static let inkPen = make { b in
        b.box(v(0, 0, -0.04), v(0.3, 0.21, 0.003), part: LIGHT, rot: M4.rotZ(0.12))
        for k in 0..<5 { b.box(v(0.08 - 0.04 * Float(k), 0.01, -0.037), v(0.004, 0.15, 0.0015), part: MID, rot: M4.rotZ(0.12)) }
        b.turn([v2(0, -0.04), v2(0.035, -0.04), v2(0.04, -0.02), v2(0.03, 0.0), v2(0.014, 0.004), v2(0.014, 0.012), v2(0, 0.012)],
               segs: 10, part: DARK, at: M4.move(v(-0.1, 0.16, 0)))
        b.rod(v(0.05, -0.03, -0.035), v(-0.09, 0.06, 0.12), 0.006, 0.008, segs: 5, part: DARK)
        b.rod(v(0.05, -0.03, -0.035), v(0.07, -0.043, -0.04), 0.003, 0.001, segs: 4, part: LIGHT)
    }

    /// A long-case clock: plinth, the trunk open in front so its pendulum shows, the hood with the
    /// dial (facing +x) at the origin, a broken pediment; the floor 1.7 below.
    static let longcaseClock = make { b in
        let f: Float = -1.7
        b.box(v(0, 0, f + 0.25), v(0.3, 0.52, 0.5), part: MID)
        b.box(v(0, 0, f + 0.52), v(0.32, 0.5, 0.04), part: MID)
        for y: Float in [-0.19, 0.19] { b.box(v(0, y, -0.73), v(0.24, 0.025, 0.95), part: MID) }
        b.box(v(-0.11, 0, -0.73), v(0.02, 0.38, 0.95), part: DARK)
        b.box(v(0.11, 0, -1.19), v(0.02, 0.38, 0.03), part: MID)
        b.box(v(0.11, 0, -0.27), v(0.02, 0.38, 0.03), part: MID)
        b.box(v(0, 0, -0.24), v(0.3, 0.48, 0.04), part: MID)
        b.box(v(0, 0, 0.02), v(0.28, 0.46, 0.5), part: MID)
        b.turn([v2(0, 0), v2(0.17, 0), v2(0.17, 0.012), v2(0, 0.012)], segs: 20, part: LIGHT, at: M4.move(v(0.141, 0, 0)) * M4.rotY(.pi / 2))
        b.hoop(0.13, 0.006, segs: 20, part: DARK, at: M4.move(v(0.155, 0, 0)) * M4.rotY(.pi / 2))
        b.box(v(0.158, 0, 0.045), v(0.004, 0.01, 0.09), part: DARK)
        b.box(v(0.158, 0.03, 0), v(0.004, 0.065, 0.008), part: DARK, rot: M4.rotX(0.5))
        for s: Float in [1, -1] {
            b.box(v(0, 0.13 * s, 0.32), v(0.3, 0.22, 0.04), part: MID, rot: M4.rotX(0.35 * s))
            b.ball(v(0.0, 0.24 * s, 0.3), 0.025, part: LIGHT)
        }
        b.ball(v(0, 0, 0.4), 0.03, part: LIGHT)
    }

    /// The pendulum, hung from its pivot at the origin.
    static let clockPendulum = make { b in
        b.rod(v(0, 0, 0), v(0, 0, -0.72), 0.006, segs: 4, part: LIGHT)
        b.turn([v2(0, -0.012), v2(0.075, -0.008), v2(0.075, 0.008), v2(0, 0.012)], segs: 18, part: LIGHT,
               at: M4.move(v(0, 0, -0.76)) * M4.rotY(.pi / 2))
    }

    /// A tall bookcase, its front to +x; the floor 1.55 below the origin.
    static let bookcase = make { b in
        let f: Float = -1.55, w: Float = 0.5, d: Float = 0.16, top: Float = 0.45
        for y in [-w, w] { b.box(v(0, y, (f + top) / 2), v(2 * d, 0.03, top - f), part: MID) }
        b.box(v(-d, 0, (f + top) / 2), v(0.02, 2 * w, top - f), part: DARK)
        var rng = SeededGenerator(41)
        var z = f + 0.08
        while z < top - 0.1 {
            b.box(v(0, 0, z), v(2 * d, 2 * w, 0.025), part: MID)
            var y = -w + 0.03
            while y < w - 0.06 {
                let t = 0.025 + 0.03 * rng.next(), h = 0.2 + 0.1 * rng.next()
                if rng.next() > 0.12 {
                    b.box(v(0.01, y + t / 2, z + 0.012 + h / 2), v(0.2, t, h), part: rng.next() > 0.6 ? LIGHT : MID)
                }
                y += t + 0.004
            }
            z += 0.38
        }
        b.box(v(0, 0, top), v(2 * d + 0.04, 2 * w + 0.06, 0.04), part: MID)
    }

    /// A cast-iron column radiator, front to +x, on its feet; the floor 0.4 below the origin.
    static let radiator = make { b in
        let f: Float = -0.4, lo: Float = -0.3, hi: Float = 0.25
        for k in 0..<11 {
            let y = -0.4 + 0.08 * Float(k)
            b.rod(v(0, y, lo), v(0, y, hi), 0.026, segs: 6, part: MID)
        }
        for z in [lo + 0.03, hi - 0.03] { b.rod(v(0, -0.42, z), v(0, 0.42, z), 0.02, segs: 6, part: MID) }
        for y: Float in [-0.36, 0.36] { b.rod(v(0, y, lo), v(0, y, f), 0.018, segs: 5, part: DARK) }
        b.rod(v(-0.04, 0.42, lo + 0.03), v(-0.04, 0.6, lo + 0.03), 0.014, segs: 5, part: DARK)
        b.rod(v(-0.04, 0.6, lo + 0.03), v(-0.04, 0.6, f), 0.014, segs: 5, part: DARK)
    }

    /// Three floorboards and their nails, lying along x, on the floor at the origin.
    static let boards = make { b in
        for k in -1...1 {
            let y = 0.15 * Float(k)
            b.box(v(0.03 * Float(k), y, 0.008), v(1.1, 0.14, 0.016), part: MID)
            for x: Float in [-0.45, 0.45] { b.ball(v(x + 0.03 * Float(k), y, 0.017), 0.01, part: DARK) }
        }
    }

    /// A panelled door ajar in its frame, front to +x; the floor 0.75 below the origin.
    static let door = make { b in
        let f: Float = -0.75, top: Float = 1.3
        for y: Float in [-0.5, 0.5] { b.box(v(0, y, (f + top) / 2), v(0.08, 0.06, top - f), part: MID) }
        b.box(v(0, 0, top), v(0.08, 1.06, 0.07), part: MID)
        // The leaf, hinged on its left post and swung a little open.
        var l = MeshBuilder()
        l.box(v(0, 0, (f + top) / 2 - 0.02), v(0.04, 0.92, top - f - 0.06), part: DARK)
        for z: Float in [-0.25, 0.65] {
            for y: Float in [-0.22, 0.22] { l.box(v(0.025, y, z), v(0.012, 0.3, 0.6), part: MID) }
        }
        b.add(l, M4.about(v(0, -0.47, 0), M4.rotZ(0.45)))
    }

    // MARK: Body scan

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

/// A person, seated: the listener in the body scan (headless: the listener's light is the head),
/// the host in the tea house. In metres, the head's centre at the origin, facing +x.
enum Figure {
    static func body(kneeling: Bool, head: Bool = true) -> MeshBuilder {
        var b = MeshBuilder()
        let P = MID
        if head { b.ball(v(0.0, 0, 0.0), 0.075, part: P, squash: v(1.0, 0.9, 1.15)) }
        b.rod(v(0, 0, -0.07), v(0, 0, -0.17), 0.04, segs: 6, part: P)
        b.turn([v2(0.11, -0.78), v2(0.13, -0.64), v2(0.12, -0.48), v2(0.14, -0.3), v2(0.11, -0.2), v2(0.04, -0.17)], segs: 10, part: P,
               at: M4.scale(v(0.8, 1.2, 1)))
        let hipZ: Float = -0.8
        for s: Float in [1, -1] {
            let shoulder = v(0, 0.17 * s, -0.24), elbow = v(0.05, 0.22 * s, -0.5)
            if kneeling {
                // Hands on the thighs.
                b.path([shoulder, elbow, v(0.28, 0.12 * s, -0.66)], 0.035, 0.03, segs: 6, part: P)
                b.path([v(0.0, 0.09 * s, hipZ), v(0.36, 0.1 * s, hipZ - 0.04), v(0.0, 0.1 * s, hipZ - 0.12)], 0.055, 0.04, segs: 6, part: P)
            } else {
                // Hands resting on the knees, legs crossed.
                let knee = v(0.3, 0.33 * s, hipZ - 0.08)
                b.path([shoulder, elbow, knee + v(0.02, -0.02 * s, 0.08)], 0.035, 0.03, segs: 6, part: P)
                b.path([v(0.02, 0.09 * s, hipZ), knee, v(0.28, -0.1 * s, hipZ - 0.14)], 0.06, 0.042, segs: 6, part: P)
            }
        }
        return b
    }
}

extension SIMD4 where Scalar == Float {
    fileprivate var xyz: SIMD3<Float> { SIMD3(x, y, z) }
}
