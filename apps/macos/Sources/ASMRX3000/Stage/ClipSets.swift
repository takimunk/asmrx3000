import Foundation
import simd

/// The place a clip scene happens in, drawn around the listener so they feel placed: the
/// lantern room of a lighthouse over the sea, a sleeper compartment, a frozen lake under the
/// stars, a station module in orbit, a tea room open to a rain garden. The place is the
/// "set": quiet linework and surfaces in their own material (`Part.set`, a pale pencil grey
/// that never takes a contour or the selection), always there, behind the sounds. The sounds'
/// models stay brighter stone and come and go as they sound.
///
/// Everything goes through the stage's radial compression (`ClipSpace.map`), so a sound at the
/// glass sits on the glass; lines are built in mapped space so they keep one drawn width near
/// and far. The wall nearest the camera is left out (a cutaway), so the room never hides the
/// listener.
enum ClipSet: Equatable {
    case none, space, underwater, lighthouse, train, ice, teaHouse, bodyScan, tent

    static func of(_ scene: SceneInfo) -> ClipSet {
        let id = scene.entry.id
        let name = ((scene.entry.file ?? id) as NSString).lastPathComponent
        func has(_ s: String) -> Bool { id.hasPrefix(s) || name.hasPrefix(s) || id.contains("/" + s) }
        if has("space") { return .space }
        if has("underwater") { return .underwater }
        if has("lighthouse") { return .lighthouse }
        if has("night_train") { return .train }
        if has("singing_ice") { return .ice }
        if has("tea_house") { return .teaHouse }
        if has("body_scan") { return .bodyScan }
        if has("mountain_tent") { return .tent }
        switch scene.look {
        case .space: return .space
        case .underwater: return .underwater
        default: return .none
        }
    }

    /// The opening framing: from behind the listener (left and right on the stage are left and
    /// right in the ears), as close and as steep as the place needs to read.
    var camera: OrbitCamera {
        var c = OrbitCamera()
        c.azimuth = .pi
        switch self {
        case .lighthouse:
            // Down into the lantern room and over the gallery to the sea below.
            c.target = SIMD3(1.2, 0, -1.2); c.elevation = 0.36; c.distance = 15
        case .train:
            // From behind the right shoulder into the cut-open compartment: the window on the left
            // wall facing the eye, the night going past beyond it.
            c.target = SIMD3(0.7, 0.0, -0.6); c.azimuth = .pi + 0.45; c.elevation = 0.42; c.distance = 9
        case .ice:
            // Low over the ice so the shore stands on the horizon.
            c.target = SIMD3(2.0, 0, -0.6); c.elevation = 0.2; c.distance = 15
        case .teaHouse:
            // From behind the left shoulder: the room, the host across, the garden on the right.
            c.target = SIMD3(1.0, -0.9, -1.0); c.azimuth = .pi - 0.38; c.elevation = 0.46; c.distance = 13
        case .bodyScan:
            c.target = SIMD3(0.4, 0, -0.6); c.elevation = 0.5; c.distance = 7
        case .tent:
            // From behind the right shoulder over the tent: the fly about the listener, the stream
            // across the meadow on the left, the valley and its ridges beyond.
            c.target = SIMD3(0.8, 0.9, -0.3); c.azimuth = .pi + 0.35; c.elevation = 0.4; c.distance = 9.5
        case .underwater:
            // Under the surface: the eye stays below it.
            c.target = SIMD3(1.2, 0, -0.6); c.elevation = 0.22; c.distance = 17
        case .space:
            c.target = SIMD3(1.0, 0, -0.4); c.elevation = 0.3; c.distance = 17
        case .none:
            c.target = SIMD3(1.2, 0, -0.4); c.elevation = 0.36; c.distance = 19
        }
        return c
    }

    /// The sky: how many stars (0...1), and whether a planet fills the lower sky.
    var stars: Float {
        switch self {
        case .space: return 1
        case .ice: return 0.8
        case .train: return 0.45
        case .bodyScan: return 0.0
        default: return 0
        }
    }

    var planet: Bool { self == .space }

    /// The station module's radius and length (world metres), shared by the set and the sounds
    /// placed on its hull.
    static let module = (radius: Float(3.3), from: Float(-5), to: Float(4.6))
    /// The compartment's partition (y) and ceiling (z), metres.
    static let partition: Float = -1.0, ceiling: Float = 1.0

    /// Where a sound is drawn: where it sounds, except the station's own creaks and ticks, put
    /// on the module's hull in their direction (the open top arc avoided), so they read as the
    /// station's.
    func drawnAt(_ kind: ClipKind, raw: SIMD3<Float>) -> SIMD3<Float> {
        if self == .tent, case .prop(.flap) = kind {
            // On the fly sheet, in the sound's direction from the tent's middle.
            let c = Self.tentCentre
            let a = atan2(raw.y - c.y, raw.x - c.x)
            let rho = min(simd_length(SIMD2(raw.x, raw.y) - c) / Self.flyRadius(a), 0.92)
            let q = c + rho * Self.flyRadius(a) * SIMD2(cos(a), sin(a))
            return SIMD3(q.x, q.y, Self.flyZ(rho))
        }
        if self == .train, case .prop(.panel) = kind {
            // On the partition to the corridor.
            return SIMD3(min(max(raw.x, -1.0), 1.4), Self.partition, min(raw.z, Self.ceiling - 0.3))
        }
        guard self == .space, case .prop(.panel) = kind else { return raw }
        let m = Self.module
        var a = atan2(raw.z, raw.y)
        let top = Float.pi / 2, half: Float = 1.15
        let off = atan2(sin(a - top), cos(a - top))
        if abs(off) < half { a = top + (off < 0 ? -half : half) }
        let x = min(max(raw.x, m.from + 0.5), m.to - 0.3)
        return SIMD3(x, m.radius * cos(a), m.radius * sin(a))
    }

    /// Whether the sounds that are part of the place (a pane, a window, the eaves) are drawn on
    /// the set rather than as things of their own.
    var hasRoom: Bool { self == .lighthouse || self == .train || self == .teaHouse || self == .tent }

    // MARK: Geometry

    /// The still part of the place, built once per scene.
    func scenery(groundZ: Float) -> [ClipPiece] {
        var k = SetKit()
        var ground = SetKit()
        var water = SetKit()
        switch self {
        case .lighthouse: Self.lighthouse(&k, &ground, &water)
        case .train: Self.train(&k, &ground)
        case .ice: Self.ice(&k, &ground, z: groundZ)
        case .teaHouse: Self.teaHouse(&k, &ground)
        case .space: Self.space(&k)
        case .bodyScan: Self.bodyScan(&k, &ground, z: groundZ)
        case .tent: Self.tent(&k, &ground, z: groundZ)
        case .underwater, .none: break
        }
        var out: [ClipPiece] = []
        for kit in [ground, water, k] {
            if !kit.b.verts.isEmpty, let m = kit.b.build() { out.append(ClipPiece(mesh: m, model: matrix_identity_float4x4)) }
        }
        return out
    }

    /// What moves with the place (the train's poles and wires going by), posed for time `t`.
    func moving(time t: Float) -> [ClipPiece] {
        guard self == .train, let box = ClipProps.setBox else { return [] }
        var out: [ClipPiece] = []
        func bar(_ a: SIMD3<Float>, _ b: SIMD3<Float>, _ w: Float) {
            out.append(ClipPiece(mesh: box, model: ClipProps.bar(ClipSpace.map(a), ClipSpace.map(b), thickness: w)))
        }
        // Telegraph poles on the left beyond the window, the wires sagging between them: going
        // back past the train at 22 m/s.
        let spacing: Float = 38, speed: Float = 22, y: Float = 7, foot: Float = -2.4, top: Float = 5.2
        let shift = (t * speed).truncatingRemainder(dividingBy: spacing)
        var tops: [[SIMD3<Float>]] = []
        for k in -4...4 {
            let x = Float(k) * spacing - shift + spacing / 2
            bar(SIMD3(x, y, foot), SIMD3(x, y, top), 0.035)
            bar(SIMD3(x, y - 0.9, top - 0.35), SIMD3(x, y + 0.9, top - 0.35), 0.03)
            tops.append([SIMD3(x, y - 0.8, top - 0.3), SIMD3(x, y + 0.8, top - 0.3)])
        }
        for i in 0..<(tops.count - 1) {
            for w in 0..<2 {
                let a = tops[i][w], b = tops[i + 1][w]
                var prev = a
                for s in 1...4 {
                    let u = Float(s) / 4
                    let q = a + (b - a) * u - SIMD3(0, 0, 1.1 * 4 * u * (1 - u))
                    bar(prev, q, 0.012)
                    prev = q
                }
            }
        }
        return out
    }

    // MARK: Places

    /// The lantern room: the lens on its pedestal at the middle, the glazing all round with its
    /// bars, the domed roof, the gallery and its railing outside, and far below the sea out to
    /// the horizon.
    private static func lighthouse(_ k: inout SetKit, _ ground: inout SetKit, _ water: inout SetKit) {
        let c = SIMD2<Float>(1.4, 0)
        let floor: Float = -1.6, sill: Float = -0.5, head: Float = 1.45, apex: Float = 2.55
        let rg: Float = 2.6, rr: Float = 3.5
        // Cut away on the camera's side (behind the listener).
        let open: (Float, Float) = (.pi, 0.95)
        ground.surface(centre: c, z: floor, from: 0, to: rr, rings: 6, part: Part.ground)
        water.surface(centre: .zero, z: -32, from: 0, to: 1600, rings: 30, part: Part.water)
        // The horizon, and a few long swells on the sea.
        k.ring(centre: .zero, radius: 1500, z: -32, w: 0.03)
        for r: Float in [90, 220, 520] { k.ring(centre: .zero, radius: r, z: -32, w: 0.008) }
        let n = 12
        func at(_ a: Float, _ r: Float, _ z: Float) -> SIMD3<Float> { SIMD3(c.x + r * cos(a), c.y + r * sin(a), z) }
        for z in [sill, head] { k.ring(centre: c, radius: rg, z: z, w: 0.02, open: open) }
        k.ring(centre: c, radius: rg, z: 0.45, w: 0.01, open: open)
        for i in 0..<n {
            let a = 2 * Float.pi * (Float(i) + 0.5) / Float(n)
            if SetKit.inside(a, open) { continue }
            k.line([at(a, rg, sill), at(a, rg, head)], 0.012)
            if i % 2 == 0 { k.line([at(a, rg, head), at(a, 1.2, apex - 0.35), at(a, 0.25, apex)], 0.012) }
        }
        k.b.ball(ClipSpace.map(SIMD3(c.x, c.y, apex + 0.15)), 0.12, part: Part.set)
        // The optic on its pedestal: always there; it turns brighter while its whir sounds.
        k.b.add(PropMeshes.lensParts, M4.move(ClipSpace.map(SIMD3(c.x, c.y, -0.4))) * M4.scale(0.75), part: Part.set)
        // The gallery outside: the deck's edge and the railing.
        k.ring(centre: c, radius: rr, z: floor, w: 0.022)
        k.ring(centre: c, radius: rr, z: floor + 1.0, w: 0.012, open: open)
        for i in 0..<24 {
            let a = 2 * Float.pi * Float(i) / 24
            if SetKit.inside(a, open) { continue }
            k.line([at(a, rr, floor), at(a, rr, floor + 1.0)], 0.009)
        }
    }

    /// A sleeper compartment, cut open on the near side: floor, the window on the left with the
    /// little table under it, the benches, the partition and its door on the right; the rest of
    /// the carriage drawn in outline fore and aft with its row of windows; outside, the
    /// embankment out to the horizon.
    private static func train(_ k: inout SetKit, _ ground: inout SetKit) {
        let floor: Float = -1.23, yl: Float = 0.62, yr: Float = -1.0, xb: Float = -1.15, xf: Float = 1.55
        ground.surface(centre: SIMD2(0, -0.8), z: -2.4, from: 3.2, to: 1400, rings: 28, part: Part.ground)
        ground.plane(x: (xb, xf), y: (yr, yl), z: floor, part: Part.section)
        // The outer wall's foot and top along the compartment, the window in it with its
        // transom and sill, the little table under it.
        k.line([SIMD3(xb, yl, floor), SIMD3(xf, yl, floor), SIMD3(xf, yl, 1.0)], 0.018)
        let lo: Float = -0.35, hi: Float = 0.5, xa: Float = -0.4, xe: Float = 0.8
        k.line([SIMD3(xa, yl, lo), SIMD3(xe, yl, lo), SIMD3(xe, yl, hi), SIMD3(xa, yl, hi), SIMD3(xa, yl, lo)], 0.03)
        k.line([SIMD3(xa, yl, 0.18), SIMD3(xe, yl, 0.18)], 0.016)
        k.line([SIMD3(0.2, yl, -0.47), SIMD3(0.2, 0.15, -0.47), SIMD3(0.9, 0.15, -0.47), SIMD3(0.9, yl, -0.47)], 0.022)
        // The partition to the corridor (its foot, top and far post) and the ceiling's edge,
        // which the creaking panel and the vent sit on.
        k.line([SIMD3(xb, yr, floor), SIMD3(xf, yr, floor), SIMD3(xf, yl, floor)], 0.016)
        k.line([SIMD3(xf, yr, floor), SIMD3(xf, yr, Self.ceiling)], 0.014)
        k.line([SIMD3(xb, yr, Self.ceiling), SIMD3(xf, yr, Self.ceiling), SIMD3(xf, yl, Self.ceiling), SIMD3(-0.4, yl, Self.ceiling)], 0.012)
    }

    /// The frozen lake to the horizon and its wooded shore all round, the shore nearer on the
    /// left where the owl is, a gap of open ice far ahead.
    private static func ice(_ k: inout SetKit, _ ground: inout SetKit, z: Float) {
        ground.surface(centre: .zero, z: z, from: 0, to: 1200, rings: 30, part: Part.ground)
        var rng = SeededGenerator(0x1CE)
        var a: Float = 0
        while a < 2 * .pi {
            let step = 0.022 + 0.02 * rng.next()
            a += step
            // Open ice far ahead-right: no trees there.
            let gap = abs(atan2(sin(a + 0.35), cos(a + 0.35))) < 0.32
            if gap { continue }
            let d = 360 + 140 * sin(2 * a + 1.1) + 70 * sin(5 * a + 0.3) + 40 * rng.next()
            let base = SIMD3(d * cos(a), d * sin(a), z - 0.5)
            let h = 11 + 13 * rng.next()
            var t = MeshBuilder()
            t.rod(base, base + SIMD3(0, 0, h), 0.11 * h, 0, segs: 4, part: Part.set)
            k.b.add(t.warped(ClipSpace.map), matrix_identity_float4x4)
        }
        // The shore's low line under the trees.
        var pts: [SIMD3<Float>] = []
        for i in 0...180 {
            let aa = 2 * Float.pi * Float(i) / 180
            let d = 360 + 140 * sin(2 * aa + 1.1) + 70 * sin(5 * aa + 0.3) + 20
            pts.append(SIMD3(d * cos(aa), d * sin(aa), z + 1.5))
        }
        k.line(pts, 0.02)
    }

    /// The tea room: tatami with their seams, the posts and the lintel, the paper screen on the
    /// left, the scroll in the alcove ahead, the veranda and the roof's edge on the right, open
    /// to the garden below with its stepping stones and a stone lantern.
    private static func teaHouse(_ k: inout SetKit, _ ground: inout SetKit) {
        let floor: Float = -0.9, lintel: Float = 1.15, veranda: Float = -0.98, garden: Float = -1.55
        let xb: Float = -1.4, xf: Float = 2.6, yl: Float = 2.2, yo: Float = -1.25, yv: Float = -2.0
        ground.plane(x: (xb, xf), y: (yo, yl), z: floor, part: Part.ground)
        ground.surface(centre: SIMD2(0.6, -1.0), z: garden, from: 2.0, to: 900, rings: 24, part: Part.ground)
        // Tatami seams (mats 0.9 x 1.8, laid in the usual pattern).
        let lift: Float = 0.012
        for x in stride(from: xb + 0.9, through: xf - 0.1, by: 0.9) {
            k.line([SIMD3(x, yo, floor + lift), SIMD3(x, yl, floor + lift)], 0.012)
        }
        for y in [yo + 1.15, yo + 2.3] { k.line([SIMD3(xb, y, floor + lift), SIMD3(xf, y, floor + lift)], 0.012) }
        k.line([SIMD3(xb, yo, floor + lift), SIMD3(xf, yo, floor + lift)], 0.022)
        // Posts, lintel and the room's edges (the near wall left out).
        for (x, y) in [(xb, yo), (xf, yo), (xb, yl), (xf, yl), (Float(0.6), yo)] {
            k.line([SIMD3(x, y, floor), SIMD3(x, y, lintel + 0.35)], 0.03)
        }
        k.line([SIMD3(xb, yo, lintel), SIMD3(xf, yo, lintel), SIMD3(xf, yl, lintel), SIMD3(xb, yl, lintel)], 0.024)
        k.line([SIMD3(xb, yo, lintel + 0.35), SIMD3(xf, yo, lintel + 0.35), SIMD3(xf, yl, lintel + 0.35), SIMD3(xb, yl, lintel + 0.35)], 0.016)
        // The paper screen on the left: its frame and lattice.
        for x in stride(from: xb + 0.25, through: xf - 0.2, by: 0.5) {
            k.line([SIMD3(x, yl, floor), SIMD3(x, yl, lintel)], 0.011)
        }
        for z in stride(from: floor + 0.35, through: lintel - 0.1, by: 0.42) {
            k.line([SIMD3(xb, yl, z), SIMD3(xf, yl, z)], 0.011)
        }
        // The alcove ahead with its hanging scroll.
        k.line([SIMD3(xf, 0.5, floor + 0.15), SIMD3(xf, 1.9, floor + 0.15)], 0.02)
        k.line([SIMD3(xf, 0.5, floor), SIMD3(xf, 0.5, lintel)], 0.022)
        k.line([SIMD3(xf - 0.02, 1.0, 0.75), SIMD3(xf - 0.02, 1.0, -0.35), SIMD3(xf - 0.02, 1.4, -0.35),
                SIMD3(xf - 0.02, 1.4, 0.75), SIMD3(xf - 0.02, 1.0, 0.75)], 0.014)
        // The veranda, its boards along the room, and the roof's edge over it.
        ground.plane(x: (xb, xf), y: (yv, yo), z: veranda, part: Part.section)
        for y in stride(from: yv, through: yo - 0.1, by: 0.25) { k.line([SIMD3(xb, y, veranda + lift), SIMD3(xf, y, veranda + lift)], 0.009) }
        for x in [xb, Float(0.6), xf] { k.line([SIMD3(x, yv, garden), SIMD3(x, yv, lintel + 0.1)], 0.026) }
        k.line([SIMD3(xb - 0.6, yv - 0.5, lintel + 0.15), SIMD3(xf + 0.6, yv - 0.5, lintel + 0.15)], 0.03)
        k.line([SIMD3(xb - 0.6, yv - 0.5, lintel + 0.15), SIMD3(xb - 0.6, yo, lintel + 0.75)], 0.02)
        k.line([SIMD3(xf + 0.6, yv - 0.5, lintel + 0.15), SIMD3(xf + 0.6, yo, lintel + 0.75)], 0.02)
        for x in stride(from: xb - 0.4, through: xf + 0.5, by: 0.5) {
            k.line([SIMD3(x, yv - 0.5, lintel + 0.15), SIMD3(x, yo + 0.2, lintel + 0.65)], 0.009)
        }
        // The garden: stepping stones to the basin, a stone lantern, the bamboo fence at the back.
        var rng = SeededGenerator(0x7EA)
        for (x, y) in [(Float(0.2), Float(-2.6)), (-0.5, -3.2), (-1.0, -3.9), (0.9, -3.4), (1.6, -4.1), (2.6, -4.6)] {
            var s = MeshBuilder()
            s.rock(seed: UInt64(1 + rng.next() * 90), bumps: 0.12, squash: SIMD3(0.28, 0.24, 0.06), part: Part.set)
            k.b.add(s.warped { ClipSpace.map($0 + SIMD3(x, y, garden)) }, matrix_identity_float4x4)
        }
        let lantern = SIMD3<Float>(-1.2, -5.6, garden)
        var l = MeshBuilder()
        l.turn([SIMD2(0.0, 0), SIMD2(0.22, 0), SIMD2(0.22, 0.08), SIMD2(0.07, 0.12), SIMD2(0.07, 0.72), SIMD2(0.2, 0.76),
                SIMD2(0.2, 1.0), SIMD2(0.36, 1.05), SIMD2(0.05, 1.3), SIMD2(0.0, 1.32)], segs: 6, part: Part.set)
        k.b.add(l.warped { ClipSpace.map($0 + lantern) }, matrix_identity_float4x4)
        for x in stride(from: Float(-5), through: 6, by: 0.4) {
            k.line([SIMD3(x, -10.5, garden), SIMD3(x, -10.5, garden + 1.6)], 0.012)
        }
        for z in [garden + 0.5, garden + 1.3] { k.line([SIMD3(-5, -10.5, z), SIMD3(6, -10.5, z)], 0.014) }
    }

    /// A pressurised module around the listener, its ribs and longerons; a truss out to the
    /// solar wings on both sides; the planet below and the stars are the engraver's (sky).
    private static func space(_ k: inout SetKit) {
        let r = Self.module.radius, xa: Float = -1.2, xe = Self.module.to
        // The module ahead of the listener, its upper arc left out so the sky stays open.
        let open: (Float, Float) = (.pi / 2, 1.1)
        for x in stride(from: xa, through: xe, by: 1.45) {
            k.ring(centre: SIMD2(x, 0), radius: r, about: .x, w: x == xe ? 0.022 : 0.011, open: open)
        }
        for i in 0..<8 {
            let a = 2 * Float.pi * (Float(i) + 0.5) / 8
            if SetKit.inside(a, open) { continue }
            k.line([SIMD3(Self.module.from, r * cos(a), r * sin(a)), SIMD3(xe, r * cos(a), r * sin(a))], 0.011)
        }
        k.ring(centre: SIMD2(xe + 0.4, 0), radius: 0.9, about: .x, w: 0.022)
        // The truss and the wings.
        for s: Float in [1, -1] {
            let y0 = r, y1: Float = 24
            for z: Float in [-0.35, 0.35] { k.line([SIMD3(-0.4, s * y0, z), SIMD3(-0.4, s * y1, z)], 0.016) }
            var y = y0
            var up = true
            while y < y1 - 0.01 {
                let y2 = min(y + 1.6, y1)
                k.line([SIMD3(-0.4, s * y, up ? -0.35 : 0.35), SIMD3(-0.4, s * y2, up ? 0.35 : -0.35)], 0.011)
                y = y2
                up.toggle()
            }
            for (ya, yb) in [(Float(9), Float(16)), (17, 24)] {
                k.plane(x: (-3.2, 2.4), y: (s > 0 ? ya : -yb, s > 0 ? yb : -ya), z: 0.0, part: Part.set)
                for x in stride(from: Float(-3.2), through: 2.4, by: 0.7) {
                    k.line([SIMD3(x, s * ya, 0.05), SIMD3(x, s * yb, 0.05)], 0.008)
                }
            }
        }
    }

    // MARK: The tent

    /// The tent's middle (the listener lies with the head near its back end, feet to +x), its
    /// floor, the fly's hem and apex heights, metres.
    static let tentCentre = SIMD2<Float>(0.4, 0)
    static let tentFloor: Float = -0.3, flyHem: Float = -0.2, flyApex: Float = 0.8

    /// The fly's hem radius from the tent's middle at angle `a`: an ellipse (1.35 x 1.15 m),
    /// drawn out into the vestibule over the door on the right (-y).
    static func flyRadius(_ a: Float) -> Float {
        let e = 1 / ((cos(a) / 1.35) * (cos(a) / 1.35) + (sin(a) / 1.15) * (sin(a) / 1.15)).squareRoot()
        return e * (1 + 0.42 * pow(max(0, -sin(a)), 6))
    }

    /// The fly's height at `rho` (0 at the apex, 1 at the hem).
    static func flyZ(_ rho: Float) -> Float {
        flyHem + (flyApex - flyHem) * pow(max(0, 1 - rho * rho), 0.6)
    }

    /// A small dome tent on an alpine meadow: the fly sheet over the listener in seams from its
    /// apex to the hem (the vestibule over the door on the right), the two crossing poles of the
    /// inner tent, its floor and door, the sleeping pad, guy lines to their pegs; a few boulders
    /// in the grass, the mountain stream across the meadow on the left, and the ridges of the
    /// valley all round, lower ahead where it opens.
    private static func tent(_ k: inout SetKit, _ ground: inout SetKit, z: Float) {
        let c = tentCentre, floor = tentFloor
        // Cut away on the camera's side (behind the right shoulder).
        let open: (Float, Float) = (.pi + 0.35, 0.95)
        ground.surface(centre: .zero, z: z, from: 0, to: 1200, rings: 28, part: Part.ground)
        ground.plane(x: (c.x - 1.1, c.x + 1.1), y: (-0.7, 0.7), z: floor + 0.006, part: Part.section)
        func fly(_ a: Float, _ rho: Float) -> SIMD3<Float> {
            let r = rho * flyRadius(a)
            return SIMD3(c.x + r * cos(a), c.y + r * sin(a), flyZ(rho))
        }
        // The fly: its hem, one fold line half way up, and the seams from the apex.
        let n = 64
        let a0 = open.0 + open.1, span = 2 * Float.pi - 2 * open.1
        k.line((0...n).map { fly(a0 + span * Float($0) / Float(n), 1) }, 0.018)
        k.line((0...n).map { fly(a0 + span * Float($0) / Float(n), 0.62) }, 0.008)
        for i in 0..<8 {
            let a = -Float.pi / 2 + 2 * Float.pi * Float(i) / 8
            if SetKit.inside(a, open) { continue }
            k.line((0...8).map { fly(a, 1 - Float($0) / 8) }, i == 0 ? 0.014 : 0.011)
        }
        k.b.ball(ClipSpace.map(SIMD3(c.x, c.y, flyApex + 0.02)), 0.05, part: Part.set)
        // Guy lines from the fly's shoulders out to their pegs.
        for a in [Float.pi / 4, 3 * .pi / 4, -.pi / 4, -3 * .pi / 4, .pi / 2] where !SetKit.inside(a, open) {
            let top = fly(a, 0.72)
            let peg = SIMD3(c.x + 1.75 * flyRadius(a) * cos(a), c.y + 1.75 * flyRadius(a) * sin(a), z)
            k.line([top, peg], 0.006)
            k.line([peg, peg + SIMD3(0, 0, 0.12)], 0.012)
        }
        // The inner tent's two poles, crossing over the listener, and its door on the right.
        let hx: Float = 1.1, hy: Float = 0.7, poleTop: Float = 0.68
        for s: Float in [1, -1] {
            let a = SIMD3(c.x - hx, -s * hy, floor), b = SIMD3(c.x + hx, s * hy, floor)
            k.line((0...16).map { i -> SIMD3<Float> in
                let u = Float(i) / 16
                return a + (b - a) * u + SIMD3(0, 0, (poleTop - floor) * sin(.pi * u))
            }, 0.014)
        }
        k.line((0...12).map { i -> SIMD3<Float> in
            let a = Float.pi * Float(i) / 12
            return SIMD3(c.x + 0.55 * cos(a), -hy + 0.16 * sin(a), floor + 0.82 * sin(a))
        }, 0.011)
        // The sleeping pad under the listener.
        let pad: [SIMD3<Float>] = [SIMD3(-0.45, -0.3, floor + 0.02), SIMD3(1.45, -0.3, floor + 0.02), SIMD3(1.45, 0.3, floor + 0.02),
                                   SIMD3(-0.45, 0.3, floor + 0.02), SIMD3(-0.45, -0.3, floor + 0.02)]
        k.line(pad, 0.012)
        for x: Float in [0.05, 0.55, 1.05] { k.line([SIMD3(x, -0.3, floor + 0.02), SIMD3(x, 0.3, floor + 0.02)], 0.006) }
        // Boulders in the meadow.
        var rng = SeededGenerator(0x7E47)
        for (x, y, r) in [(Float(6), Float(-4.5), Float(0.5)), (-3.5, -6.5, 0.35), (10, 4, 0.7), (14, -9, 0.9), (-9, -3, 0.6), (22, 6, 1.2),
                          (4.5, 9.5, 0.45), (-5, 11, 0.6)] as [(Float, Float, Float)] {
            var s = MeshBuilder()
            s.rock(seed: UInt64(1 + rng.next() * 90), bumps: 0.14, squash: SIMD3(r, r * 0.85, r * 0.55), part: Part.set)
            k.b.add(s.warped { ClipSpace.map($0 + SIMD3(x, y, z)) }, matrix_identity_float4x4)
        }
        // The stream across the meadow on the left: its two banks, a few riffles, stones in it.
        func mid(_ x: Float) -> Float { 14 + 1.8 * sin(x / 9 + 0.5) + 0.7 * sin(x / 3.7) }
        let xs = Array(stride(from: Float(-60), through: 80, by: 1.5))
        for side: Float in [1, -1] { k.line(xs.map { SIMD3($0, mid($0) + side * 1.1, z + 0.02) }, 0.014) }
        for x0 in stride(from: Float(-14), through: 22, by: 4.5) {
            let w = 0.35 * sin(x0 * 1.3)
            k.line((0...4).map { i in let x = x0 + Float(i) * 0.5; return SIMD3(x, mid(x) + w, z + 0.03) }, 0.007)
        }
        for x in stride(from: Float(-10), through: 18, by: 3.5) {
            var s = MeshBuilder()
            let r = 0.18 + 0.2 * rng.next()
            s.rock(seed: UInt64(1 + rng.next() * 90), bumps: 0.14, squash: SIMD3(r * 1.3, r, r * 0.5), part: Part.set)
            let at = SIMD3(x + rng.next(), mid(x) + 1.6 * (rng.next() - 0.5), z)
            k.b.add(s.warped { ClipSpace.map($0 + at) }, matrix_identity_float4x4)
        }
        // The ridges of the valley: high on both sides, lower ahead where it opens out.
        for (d, w) in [(Float(950), Float(0.02)), (1150, 0.012)] {
            var pts: [SIMD3<Float>] = []
            for i in 0...240 {
                let a = -Float.pi + 2 * Float.pi * Float(i) / 240
                let s = sin(a)
                let h = 70 + 260 * s * s * (1 - 0.3 * cos(a)) + 40 * sin(7 * a + 1 + d) + 22 * sin(13 * a + d)
                let r = d + 90 * sin(3 * a)
                pts.append(SIMD3(r * cos(a), r * sin(a), z + h * (d > 1000 ? 1.25 : 1)))
            }
            k.line(pts, w)
        }
    }

    /// A round mat under the listener on a dark floor, one quiet ring about them.
    private static func bodyScan(_ k: inout SetKit, _ ground: inout SetKit, z: Float) {
        ground.surface(centre: .zero, z: z, from: 0, to: 9, rings: 10, part: Part.ground)
        k.surface(centre: .zero, z: z + 0.02, from: 0, to: 0.9, rings: 4, part: Part.section)
        k.ring(centre: .zero, radius: 0.92, z: z + 0.03, w: 0.02)
    }
}

/// Builds the set in the stage's mapped space.
struct SetKit {
    var b = MeshBuilder()

    enum Axis { case x, z }

    /// A line through world points, bent with the map (each piece short against its distance
    /// from the head), `w` thick in mapped units whatever its distance.
    mutating func line(_ pts: [SIMD3<Float>], _ w: Float = 0.018, part: Float = Part.set) {
        guard pts.count > 1 else { return }
        var out: [SIMD3<Float>] = []
        for i in 0..<(pts.count - 1) {
            let a = pts[i], c = pts[i + 1]
            let near = max(min(simd_length(a), simd_length(c)), 0.4)
            let n = max(1, min(40, Int((simd_distance(a, c) / (0.3 * near)).rounded(.up))))
            for s in 0..<n { out.append(ClipSpace.map(a + (c - a) * Float(s) / Float(n))) }
        }
        out.append(ClipSpace.map(pts[pts.count - 1]))
        b.path(out, w, segs: 4, part: part)
    }

    /// A circle about z (at height z) or about x (at x = centre.x, in the y-z plane).
    /// `open` leaves out an arc (centre angle, half width): the side towards the camera.
    mutating func ring(centre c: SIMD2<Float>, radius r: Float, z: Float = 0, about axis: Axis = .z, w: Float = 0.018,
                       open: (Float, Float)? = nil) {
        let n = 48
        let a0 = open.map { $0.0 + $0.1 } ?? 0
        let span = 2 * Float.pi - 2 * (open?.1 ?? 0)
        let pts = (0...n).map { i -> SIMD3<Float> in
            let a = a0 + span * Float(i) / Float(n)
            return axis == .z ? SIMD3(c.x + r * cos(a), c.y + r * sin(a), z) : SIMD3(c.x, r * cos(a), r * sin(a))
        }
        line(pts, w)
    }

    /// Whether angle `a` is in the open arc.
    static func inside(_ a: Float, _ open: (Float, Float)) -> Bool {
        abs(atan2(sin(a - open.0), cos(a - open.0))) < open.1
    }

    /// A level disc or annulus about `centre`, rings spaced geometrically out to `to`, mapped.
    mutating func surface(centre c: SIMD2<Float>, z: Float, from r0: Float, to r1: Float, rings: Int, part: Float) {
        let segs = 48
        var radii: [Float] = []
        let first = max(r0, 0.25)
        let q = pow(r1 / first, 1 / Float(max(rings - 1, 1)))
        var r = first
        for _ in 0..<rings { radii.append(r); r *= q }
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        let centreCap = r0 <= 0.01
        if centreCap { pos.append(SIMD3(c.x, c.y, z)) }
        let base = centreCap ? 1 : 0
        for rr in radii {
            for k in 0..<segs {
                let a = 2 * Float.pi * Float(k) / Float(segs)
                pos.append(SIMD3(c.x + rr * cos(a), c.y + rr * sin(a), z))
            }
        }
        func at(_ r: Int, _ k: Int) -> UInt32 { UInt32(base + r * segs + k % segs) }
        if centreCap { for k in 0..<segs { tri += [0, at(0, k), at(0, k + 1)] } }
        for r in 0..<(radii.count - 1) {
            for k in 0..<segs { tri += [at(r, k), at(r + 1, k), at(r, k + 1), at(r, k + 1), at(r + 1, k), at(r + 1, k + 1)] }
        }
        var m = MeshBuilder()
        m.smooth(pos, tri, part: part)
        b.add(m.warped(ClipSpace.map), matrix_identity_float4x4)
    }

    /// A level rectangle, gridded finely enough to bend with the map.
    mutating func plane(x: (Float, Float), y: (Float, Float), z: Float, part: Float) {
        let nx = max(2, Int(abs(x.1 - x.0) / 0.35)), ny = max(2, Int(abs(y.1 - y.0) / 0.35))
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        for j in 0...ny {
            for i in 0...nx {
                pos.append(SIMD3(x.0 + (x.1 - x.0) * Float(i) / Float(nx), y.0 + (y.1 - y.0) * Float(j) / Float(ny), z))
            }
        }
        for j in 0..<ny {
            for i in 0..<nx {
                let a = UInt32(j * (nx + 1) + i), bb = a + 1, c = a + UInt32(nx + 1), d = c + 1
                tri += [a, bb, c, bb, d, c]
            }
        }
        var m = MeshBuilder()
        m.smooth(pos, tri, part: part)
        b.add(m.warped(ClipSpace.map), matrix_identity_float4x4)
    }
}
