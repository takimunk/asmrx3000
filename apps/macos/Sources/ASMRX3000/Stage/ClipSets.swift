import Foundation
import simd

/// The place a clip scene happens in, drawn around the listener so they feel placed: the
/// lantern room of a lighthouse over the sea, a sleeper compartment, a frozen lake under the
/// stars, a station module in orbit, a tea room open to a rain garden, a yacht's cabin at anchor
/// in a bay. The place is the
/// "set": quiet linework and surfaces in their own material (`Part.set`, a pale pencil grey
/// that never takes a contour or the selection), always there, behind the sounds. The sounds'
/// models stay brighter stone and come and go as they sound.
///
/// Everything goes through the stage's radial compression (`ClipSpace.map`), so a sound at the
/// glass sits on the glass; lines are built in mapped space so they keep one drawn width near
/// and far. The wall nearest the camera is left out (a cutaway), so the room never hides the
/// listener.
enum ClipSet: Equatable {
    case none, space, underwater, lighthouse, train, ice, teaHouse, bodyScan, cave, yacht, beach

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
        if has("cave_drips") { return .cave }
        if has("sailboat") { return .yacht }
        if has("campfire_shore") { return .beach }
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
        case .cave:
            // From behind, a little above the ledge: the floor and its pools below, the vault over all.
            c.target = SIMD3(1.6, 0, -1.3); c.elevation = 0.4; c.distance = 15
        case .yacht:
            // From over the stern: the cabin under the deck, the mast going up through it, the
            // bow and its chain ahead, the bay around.
            c.target = SIMD3(0.6, -0.9, 1.2); c.azimuth = .pi + 0.35; c.elevation = 0.34; c.distance = 22
        case .beach:
            // From behind the driftwood seat, over the fire to the waterline and the sea.
            c.target = SIMD3(2.4, 0, -0.7); c.elevation = 0.3; c.distance = 7.5
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
        case .yacht: return 0.6
        case .bodyScan: return 0.0
        case .beach: return 0.75
        default: return 0
        }
    }

    var planet: Bool { self == .space }

    /// The station module's radius and length (world metres), shared by the set and the sounds
    /// placed on its hull.
    static let module = (radius: Float(3.3), from: Float(-5), to: Float(4.6))
    /// The compartment's partition (y) and ceiling (z), metres.
    static let partition: Float = -1.0, ceiling: Float = 1.0
    /// The yacht about the listener in its port berth (core/scenes/sailboat_anchor.json):
    /// centreline (y), waterline, deck and keel (z), bow and transom (x), the mast's foot.
    static let boat = (centre: Float(-0.9), halfBeam: Float(1.55), waterline: Float(-0.35), deck: Float(0.55),
                        keel: Float(-1.3), bow: Float(5.0), stern: Float(-4.6), mast: SIMD2<Float>(1.2, -0.9))

    /// Where a sound is drawn: where it sounds, except the station's own creaks and ticks, put
    /// on the module's hull in their direction (the open top arc avoided), so they read as the
    /// station's.
    func drawnAt(_ kind: ClipKind, raw: SIMD3<Float>) -> SIMD3<Float> {
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
    var hasRoom: Bool { self == .lighthouse || self == .train || self == .teaHouse }

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
        case .cave: Self.cave(&k, &ground, &water, z: groundZ)
        case .yacht: Self.yacht(&k, &ground, &water)
        case .beach: Self.beach(&k, &ground, &water, z: groundZ)
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

    /// The cave's pools (x, y, radius): where the plinks fall (cave_drips.json, groups pools and deep).
    static let cavePools: [(Float, Float, Float)] = {
        let ring: [(Float, Float, Float)] = [(35, 5.2, 1.1), (-40, 8.5, 1.5), (120, 6.5, 1.3), (-70, 4.0, 0.9), (10, 13.0, 2.0), (-145, 7.5, 1.4)]
        return ring.map { (az, r, size) in (r * cos(az * .pi / 180), r * sin(az * .pi / 180), size) } + [(16, -7, 2.4), (11, 12, 2.2)]
    }()
    /// Where drops land on rock (the rock group's places), for the stalactites above them.
    static let caveRockDrips: [SIMD3<Float>] = [SIMD3(1.0, 1.1, -0.9), SIMD3(-1.5, 2.6, 2.8), SIMD3(2.6, -1.8, 4.6),
                                               SIMD3(-4.0, -3.0, 1.0), SIMD3(5.5, 5.5, 1.6), SIMD3(-2.2, 0.4, 5.8)]
    /// The ledge you sit on (its top, and its x/y extent).
    static let caveLedge = (z: Float(-0.9), x: (Float(-1.3), Float(1.25)), y: (Float(-1.3), Float(1.4)))

    /// The vault over the cave about (2, 0): its wall radius at each height, floor to crown.
    private static let vault: [(r: Float, z: Float)] = [(16, -3.6), (15.4, 0), (13.5, 3), (10.5, 5.5), (7, 7.5), (3.2, 8.8), (0, 9.2)]

    /// Height of the vault's underside above the point (x, y).
    static func caveCeiling(_ x: Float, _ y: Float) -> Float {
        let r = ((x - 2) * (x - 2) + y * y).squareRoot()
        for i in 0..<(vault.count - 1) where r <= vault[i].r && r >= vault[i + 1].r {
            let u = (vault[i].r - r) / max(vault[i].r - vault[i + 1].r, 1e-3)
            return vault[i].z + (vault[i + 1].z - vault[i].z) * u
        }
        return vault[0].z
    }

    /// A limestone hall: the ledge you sit on with its edge falling to the floor, the floor and
    /// its still pools, the stream along the right, the vault drawn in contours and ribs (open
    /// behind, where the passage runs back to the entrance), stalactites over the drips.
    private static func cave(_ k: inout SetKit, _ ground: inout SetKit, _ water: inout SetKit, z floor: Float) {
        let c = SIMD2<Float>(2, 0)
        let L = caveLedge
        ground.surface(centre: c, z: floor, from: 0, to: 16, rings: 12, part: Part.ground)
        // The ledge: its top, the rim, and the rock face down to the floor in a few strokes.
        ground.plane(x: L.x, y: L.y, z: L.z, part: Part.ground)
        k.line([SIMD3(L.x.0, L.y.0, L.z), SIMD3(L.x.1, L.y.0, L.z), SIMD3(L.x.1, L.y.1, L.z), SIMD3(L.x.0, L.y.1, L.z)], 0.02)
        var rng = SeededGenerator(0xCA7E)
        for i in 0...6 {
            let y = L.y.0 + (L.y.1 - L.y.0) * Float(i) / 6
            let lean = 0.5 + 0.5 * rng.next()
            k.line([SIMD3(L.x.1, y, L.z), SIMD3(L.x.1 + 0.3 * lean, y + 0.1, (L.z + floor) / 2), SIMD3(L.x.1 + 0.6 * lean, y, floor)], 0.011)
        }
        for x in [L.x.0, Float(0), L.x.1 - 0.3] {
            for y in [L.y.0, L.y.1] {
                let out: Float = y > 0 ? 0.5 : -0.5
                k.line([SIMD3(x, y, L.z), SIMD3(x, y + out, floor)], 0.01)
            }
        }
        // The small basin by the right knee that the slow drip falls into.
        water.surface(centre: SIMD2(0.75, -0.85), z: L.z + 0.01, from: 0, to: 0.22, rings: 2, part: Part.water)
        // The pools on the floor, each with its rim.
        for (x, y, r) in cavePools {
            water.surface(centre: SIMD2(x, y), z: floor + 0.03, from: 0, to: r, rings: 3, part: Part.water)
            k.ring(centre: SIMD2(x, y), radius: r, z: floor + 0.04, w: 0.012)
        }
        // The stream along the floor on the right, from behind to far ahead, between its banks.
        let course: [SIMD2<Float>] = [SIMD2(-10, -4.4), SIMD2(-4, -5.4), SIMD2(1, -6), SIMD2(5, -7.4), SIMD2(9, -10), SIMD2(14, -12), SIMD2(22, -13.5)]
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        var banks: [[SIMD3<Float>]] = [[], []]
        for (i, q) in course.enumerated() {
            let a = course[max(i - 1, 0)], b = course[min(i + 1, course.count - 1)]
            let d = simd_normalize(b - a)
            let n = SIMD2(-d.y, d.x) * (0.55 + 0.04 * Float(i))
            for (j, s) in [Float(1), -1].enumerated() {
                let p = SIMD3(q.x + n.x * s, q.y + n.y * s, floor + 0.04)
                pos.append(p)
                banks[j].append(p + SIMD3(0, 0, 0.02))
            }
            if i > 0 {
                let o = UInt32(2 * (i - 1))
                tri += [o, o + 2, o + 1, o + 1, o + 2, o + 3]
            }
        }
        var ribbon = MeshBuilder()
        ribbon.smooth(pos, tri, part: Part.water)
        water.b.add(ribbon.warped(ClipSpace.map), matrix_identity_float4x4)
        for bank in banks { k.line(bank, 0.012) }
        // The vault: contours at a few heights and ribs from the floor to the crown, left open
        // behind the listener (the camera's side and the passage to the entrance).
        let open: (Float, Float) = (.pi, 0.75)
        for v in vault.dropFirst().dropLast() { k.ring(centre: c, radius: v.r, z: v.z, w: v.z > 6 ? 0.012 : 0.016, open: open) }
        for i in 0..<14 {
            let a = 2 * Float.pi * (Float(i) + 0.5) / 14
            if SetKit.inside(a, open) { continue }
            let wob = 0.6 * sin(Float(i) * 2.3)
            k.line(vault.map { v in SIMD3(c.x + (v.r + (v.r > 1 ? wob : 0)) * cos(a), c.y + (v.r + (v.r > 1 ? wob : 0)) * sin(a), v.z) }, 0.01)
        }
        // Stalactites over the pools and the drops on rock, and a scatter of smaller ones.
        var drops: [SIMD3<Float>] = cavePools.map { SIMD3($0.0, $0.1, 0) } + caveRockDrips
        for _ in 0..<10 {
            let a = 2 * Float.pi * rng.next(), r = 2 + 11 * rng.next()
            let p = SIMD3(c.x + r * cos(a), c.y + r * sin(a), 0)
            if !SetKit.inside(atan2(p.y, p.x), open) { drops.append(p) }
        }
        var tips = MeshBuilder()
        for (i, d) in drops.enumerated() {
            let top = caveCeiling(d.x, d.y)
            let len: Float = i < cavePools.count ? 1.6 : 0.6 + 1.0 * rng.next()
            tips.rod(SIMD3(d.x, d.y, top + 0.3), SIMD3(d.x, d.y, top - len), 0.16 + 0.06 * len, 0.0, segs: 5, part: Part.set)
        }
        k.b.add(tips.warped(ClipSpace.map), matrix_identity_float4x4)
        // Boulders on the floor.
        for _ in 0..<9 {
            let a = 2 * Float.pi * rng.next(), r = 3 + 10 * rng.next()
            let at = SIMD3(c.x + r * cos(a), c.y + r * sin(a), floor)
            var s = MeshBuilder()
            s.rock(seed: UInt64(1 + rng.next() * 90), bumps: 0.2, squash: SIMD3(0.5, 0.45, 0.3) * (0.6 + rng.next()), part: Part.set)
            k.b.add(s.warped { ClipSpace.map($0 + at) }, matrix_identity_float4x4)
        }
        // The passage behind: its floor running back towards the entrance.
        for y: Float in [-2.4, 2.4] { k.line([SIMD3(-11, y * 1.2, floor), SIMD3(-22, y, floor + 0.6), SIMD3(-40, y * 0.8, floor + 1.4)], 0.014) }
    }

    /// The yacht at anchor: its hull in outline (sheer, waterline, keel, transom), the cabin sole
    /// and the berth the listener lies in, the coachroof, the mast through the cabin with its
    /// spreaders, shrouds and stays, the boom; the chain from the bow down to the anchor on the
    /// sand; the bay's water out to the horizon and its low shore on three sides.
    private static func yacht(_ k: inout SetKit, _ ground: inout SetKit, _ water: inout SetKit) {
        let y = Self.boat
        let c = y.centre, wl = y.waterline
        // Half beam along the boat: full aft of amidships, a fine bow.
        func half(_ x: Float) -> Float {
            let u = (x - y.stern) / (y.bow - y.stern)
            return y.halfBeam * (u < 0.55 ? 0.82 + 0.18 * sin(Float.pi / 2 * u / 0.55) : max(cos(Float.pi / 2 * (u - 0.55) / 0.45), 0))
        }
        func outline(_ z: Float, _ scale: Float, _ w: Float, from x0: Float? = nil) {
            let xs = stride(from: x0 ?? y.stern, through: y.bow, by: 0.35).map { $0 } + [y.bow]
            for s: Float in [1, -1] { k.line(xs.map { SIMD3($0, c + s * scale * half($0), z) }, w) }
        }
        // The sheer (deck edge), the waterline, the bilge between, and the transom.
        outline(y.deck, 1, 0.02)
        outline(wl, 0.94, 0.012)
        outline((wl + y.keel) / 2, 0.7, 0.008)
        k.line([SIMD3(y.stern, c + half(y.stern), y.deck), SIMD3(y.stern, c + 0.94 * half(y.stern), wl),
                SIMD3(y.stern + 0.4, c, y.keel + 0.25), SIMD3(y.stern, c - 0.94 * half(y.stern), wl),
                SIMD3(y.stern, c - half(y.stern), y.deck)], 0.016)
        // The keel line and the fin keel under it.
        k.line([SIMD3(y.stern + 0.4, c, y.keel + 0.25), SIMD3(-1.0, c, y.keel), SIMD3(3.2, c, y.keel + 0.1),
                SIMD3(y.bow, c, y.deck)], 0.014)
        k.line([SIMD3(-0.4, c, y.keel), SIMD3(-0.7, c, y.keel - 1.1), SIMD3(0.5, c, y.keel - 1.1), SIMD3(1.0, c, y.keel)], 0.014)
        // Inside: the cabin sole, the berth under the listener, the bulkhead ahead.
        ground.plane(x: (-2.2, 2.4), y: (c - 0.45, c + 0.45), z: -0.95, part: Part.section)
        k.plane(x: (-1.4, 0.75), y: (0.0, 0.6), z: -0.22, part: Part.section)
        k.line([SIMD3(2.4, c - 1.2, -0.95), SIMD3(2.4, c - 1.2, y.deck), SIMD3(2.4, c + 1.3, y.deck), SIMD3(2.4, c + 1.3, -0.95)], 0.012)
        // The coachroof over the cabin and its portlights.
        let roof: Float = 0.95, rx0: Float = -1.9, rx1: Float = 2.7, rw: Float = 0.95
        k.line([SIMD3(rx0, c + rw, roof), SIMD3(rx1, c + rw * 0.7, roof), SIMD3(rx1, c - rw * 0.7, roof),
                SIMD3(rx0, c - rw, roof), SIMD3(rx0, c + rw, roof)], 0.016)
        for s: Float in [1, -1] {
            k.line([SIMD3(rx0, c + s * rw, roof), SIMD3(rx0, c + s * rw * 1.1, y.deck)], 0.01)
            k.line([SIMD3(rx1, c + s * rw * 0.7, roof), SIMD3(rx1 + 0.3, c + s * rw * 0.8, y.deck)], 0.01)
            for x: Float in [-1.1, 0.0, 1.1] {
                k.line([SIMD3(x - 0.3, c + s * rw * 1.04, 0.72), SIMD3(x + 0.3, c + s * rw * 1.0, 0.72)], 0.024)
            }
        }
        // The mast from its step on the keel through the cabin, spreaders, shrouds, stays, boom.
        let m = y.mast, top: Float = 11.5, spr: Float = 6.2
        k.line([SIMD3(m.x, m.y, y.keel + 0.1), SIMD3(m.x, m.y, top)], 0.03)
        k.line([SIMD3(m.x, m.y - 1.0, spr), SIMD3(m.x, m.y + 1.0, spr)], 0.016)
        for s: Float in [1, -1] {
            k.line([SIMD3(m.x, m.y, top), SIMD3(m.x, m.y + s * 1.0, spr), SIMD3(m.x - 0.2, c + s * half(m.x - 0.2), y.deck)], 0.008)
        }
        k.line([SIMD3(m.x, m.y, top - 0.2), SIMD3(y.bow, c, y.deck + 0.1)], 0.008)
        k.line([SIMD3(m.x, m.y, top), SIMD3(y.stern + 0.1, c, y.deck + 0.1)], 0.008)
        k.line([SIMD3(m.x, m.y, 1.6), SIMD3(-2.9, c, 1.7)], 0.026)
        // The chain from the bow roller down to the anchor on the sand, and the anchor.
        var chain: [SIMD3<Float>] = []
        let a0 = SIMD3<Float>(y.bow + 0.1, c, 0.3), a1 = SIMD3<Float>(y.bow + 16, c, -6.0)
        for i in 0...12 {
            let u = Float(i) / 12
            let p = a0 + (a1 - a0) * u
            chain.append(SIMD3(p.x, p.y, min(p.z, a0.z + (a1.z - a0.z) * (1 - (1 - u) * (1 - u)) - 0.4 * sin(Float.pi * u))))
        }
        k.line(chain, 0.012)
        k.line([a1 + SIMD3(0, -0.6, 0), a1 + SIMD3(0.3, 0, 0), a1 + SIMD3(0, 0.6, 0)], 0.02)
        // The bay: the water from beside the hull to the horizon, low swells, the shore round.
        water.surface(centre: SIMD2(0.2, c), z: wl, from: 0, to: 1500, rings: 30, part: Part.water)
        for sc: Float in [1.25, 1.6] { outline(wl, sc, 0.006, from: y.stern - 0.6) }
        k.ring(centre: .zero, radius: 1400, z: wl, w: 0.03)
        for r: Float in [40, 120, 320] { k.ring(centre: SIMD2(0.2, c), radius: r, z: wl, w: 0.007) }
        var rng = SeededGenerator(0xBA7)
        var shore: [SIMD3<Float>] = []
        var a: Float = 0.25
        while a < 4.6 {
            let d = 520 + 140 * sin(1.7 * a + 0.4) + 50 * rng.next()
            let h = 14 + 22 * max(sin(2.3 * a + 0.8), 0) + 6 * rng.next()
            shore.append(SIMD3(d * cos(a), d * sin(a), wl + h))
            a += 0.05
        }
        k.line(shore, 0.02)
        k.line(shore.map { SIMD3($0.x, $0.y, wl + 0.5) }, 0.012)
    }

    /// A round mat under the listener on a dark floor, one quiet ring about them.
    private static func bodyScan(_ k: inout SetKit, _ ground: inout SetKit, z: Float) {
        ground.surface(centre: .zero, z: z, from: 0, to: 9, rings: 10, part: Part.ground)
        k.surface(centre: .zero, z: z + 0.02, from: 0, to: 0.9, rings: 4, part: Part.section)
        k.ring(centre: .zero, radius: 0.92, z: z + 0.03, w: 0.02)
    }
}

extension ClipSet {
    /// The waterline of the campfire beach: 14 m ahead, curving gently away to both sides (a
    /// wide bay), so the sea fills the front arc and the sand runs back to the dunes.
    static func shore(_ y: Float) -> Float { 14 + 0.0035 * y * y }
    /// The sea's surface: a little below the sand where the listener sits (ground_z -0.85); the
    /// scene's sea sounds sit just above it, since the engine's ground is one flat plane.
    static let seaZ: Float = -0.97

    /// Night on a pebble beach: sand from the dunes behind down to the waterline, the sea beyond
    /// it to the horizon with two lines of foam along the shore, the driftwood log the listener
    /// sits on, a few pebbles, and the dunes behind with tufts of marram along their crest.
    fileprivate static func beach(_ k: inout SetKit, _ ground: inout SetKit, _ water: inout SetKit, z: Float) {
        // Rows along the shore (y) and away from it (x), spaced geometrically so the grid stays fine
        // near the listener and coarse far out; built in world metres, then mapped.
        let ys = (-24...24).map { i -> Float in
            let u = Float(i) / 24
            return 320 * u * u * u + 6 * u
        }
        func sheet(_ rows: (Float) -> [SIMD3<Float>], part: Float, into kit: inout SetKit) {
            var pos: [SIMD3<Float>] = []
            var tri: [UInt32] = []
            var width = 0
            for y in ys {
                let r = rows(y)
                width = r.count
                pos += r
            }
            for j in 0..<(ys.count - 1) {
                for i in 0..<(width - 1) {
                    let a = UInt32(j * width + i), b = a + 1, c = a + UInt32(width), d = c + 1
                    tri += [a, c, b, b, c, d]
                }
            }
            var m = MeshBuilder()
            m.smooth(pos, tri, part: part)
            kit.b.add(m.warped(ClipSpace.map), matrix_identity_float4x4)
        }
        // Sand: from 40 m behind to the waterline, sloping gently down to it.
        sheet({ y in
            let x1 = shore(y)
            return (0...28).map { i -> SIMD3<Float> in
                let u = Float(i) / 28
                let x = -40 + (x1 + 40) * (1 - (1 - u) * (1 - u))
                let slope = max(0, (x - 4) / (x1 - 4))
                return SIMD3(x, y, z + (seaZ - z) * slope * slope)
            }
        }, part: Part.ground, into: &ground)
        // The sea, from the waterline out to the horizon.
        sheet({ y in
            let x0 = shore(y)
            return (0...14).map { i -> SIMD3<Float> in
                let u = Float(i) / 14
                return SIMD3(x0 + (1500 - x0) * u * u * u, y, seaZ)
            }
        }, part: Part.water, into: &water)
        // The waterline and two lines of foam, the horizon.
        for (off, w) in [(Float(0), Float(0.026)), (2.5, 0.012), (6.5, 0.008)] {
            k.line(stride(from: Float(-60), through: 60, by: 3).map { y in SIMD3(shore(y) + off, y, seaZ + 0.02) }, w)
        }
        k.ring(centre: .zero, radius: 1500, z: seaZ, w: 0.03, open: (.pi, 1.45))
        // The driftwood log the listener sits on, a branch stub, and pebbles about the fire.
        k.line([SIMD3(-0.45, 1.0, z + 0.12), SIMD3(-0.42, 0, z + 0.15), SIMD3(-0.4, -0.9, z + 0.12)], 0.028)
        k.line([SIMD3(-0.42, -0.55, z + 0.16), SIMD3(-0.3, -0.72, z + 0.3)], 0.012)
        var rng = SeededGenerator(0xB3AC)
        for _ in 0..<14 {
            let a = 2 * Float.pi * rng.next(), r = 1.6 + 5.5 * rng.next()
            let p = SIMD3(1.0 + r * cos(a) * 0.8, r * sin(a), z)
            var s = MeshBuilder()
            let d = 0.05 + 0.07 * rng.next()
            s.rock(seed: UInt64(3 + rng.next() * 90), bumps: 0.15, squash: SIMD3(d, d * 0.8, d * 0.45), part: Part.set)
            k.b.add(s.warped { ClipSpace.map($0 + p) }, matrix_identity_float4x4)
        }
        // The dunes behind: three low crests, marram along the nearest.
        for (x, h, w) in [(Float(-6), Float(0.9), Float(0.016)), (-11, 1.6, 0.012), (-18, 2.4, 0.01)] {
            k.line(stride(from: Float(-40), through: 40, by: 2).map { y in
                SIMD3(x - 0.002 * y * y + 0.6 * sin(y * 0.21), y, z + h * (0.75 + 0.25 * sin(y * 0.13 + x)))
            }, w)
        }
        for i in 0..<34 {
            let y = -13 + Float(i) * 0.8 + 0.3 * rng.next()
            let base = SIMD3(-6 - 0.002 * y * y + 0.6 * sin(y * 0.21), y, z + 0.9 * (0.75 + 0.25 * sin(y * 0.13 - 6)))
            for b in 0..<3 {
                let lean = SIMD3(0.12 * (rng.next() - 0.5) + 0.12, 0.25 * (Float(b) - 1), 0.55 + 0.25 * rng.next())
                k.line([base, base + lean], 0.006)
            }
        }
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
