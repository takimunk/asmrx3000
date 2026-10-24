import CAsmr
import Foundation
import simd

/// Clip scenes (underwater, space…) go through the same stage as the forest (SceneRenderer:
/// shadows, G-buffer, engraving), with their own things instead of trees and the river: each
/// sound a stone body where it is (bigger for beds, smaller for creatures and events), small
/// beads behind the moving ones (their trail), the bird rings while one sounds, the screen as
/// a stone frame in front of the listener, and underwater the bottom. See `ClipProps`.
///
/// Distances run from 0.6 m (the screen) to ~100 m (a whale passing), so the stage is drawn in
/// a radially compressed space around the head (`ClipSpace.map`): directions are true, near
/// things get room, far ones stay in view. Each scene's place and framing: `ClipSet`.
enum ClipSpace {
    /// Mapped radius: linear (× `zoom`) out to `linear` metres, so a room about the listener
    /// keeps straight walls; beyond it logarithmic, joining smoothly (same slope at the seam).
    /// 1 m → 1.5, 4 m → 6.0, 10 m → 8.6, 30 m → 10.2, 100 m → 11.8, 1 km → 14.6.
    static let linear: Float = 4, zoom: Float = 1.5, far: Float = 1.2
    static func radius(_ r: Float) -> Float {
        if r <= linear { return r * zoom }
        let knee = far / zoom
        return linear * zoom + far * log(1 + (r - linear) / knee)
    }

    static func map(_ p: SIMD3<Float>) -> SIMD3<Float> {
        let r = simd_length(p)
        guard r > 1e-4 else { return p }
        return p * (radius(r) / r)
    }
}

/// The meshes and instances of a clip scene, in SceneRenderer's terms.
enum ClipProps {
    /// A unit sphere (radius 1), stone.
    static let sphere: Mesh? = {
        var v: [MeshVertex] = []
        var idx: [UInt32] = []
        let rings = 10, segs = 16
        for r in 0...rings {
            let th = Float.pi * Float(r) / Float(rings)
            for s in 0...segs {
                let ph = 2 * Float.pi * Float(s) / Float(segs)
                let n = SIMD3(sin(th) * cos(ph), sin(th) * sin(ph), cos(th))
                v.append(MeshVertex(n, n, Part.stone))
            }
        }
        for r in 0..<rings {
            for s in 0..<segs {
                let a = UInt32(r * (segs + 1) + s), b = a + 1, c = a + UInt32(segs + 1), d = c + 1
                idx += [a, c, b, b, c, d]
            }
        }
        return Mesh.make(v, idx)
    }()

    /// A unit cube (-0.5...0.5), stone: bars of the screen's frame and a sweep's wall.
    static let box: Mesh? = {
        var v: [MeshVertex] = []
        var idx: [UInt32] = []
        let axes: [SIMD3<Float>] = [SIMD3(1, 0, 0), SIMD3(0, 1, 0), SIMD3(0, 0, 1)]
        for a in axes {
            for sign: Float in [1, -1] {
                let n = a * sign
                let u = SIMD3(a.y, a.z, a.x), w = simd_cross(n, u)
                let base = UInt32(v.count)
                for (i, j) in [(-1, -1), (1, -1), (1, 1), (-1, 1)] as [(Float, Float)] {
                    v.append(MeshVertex(0.5 * (n + u * i + w * j), n, Part.stone))
                }
                idx += [base, base + 1, base + 2, base, base + 2, base + 3]
            }
        }
        return Mesh.make(v, idx)
    }()

    /// A unit cube in the set's material (the train's passing poles and wires).
    static let setBox: Mesh? = {
        var b = MeshBuilder()
        b.add(Prim.box, matrix_identity_float4x4, part: Part.set)
        return b.build()
    }()

    /// The bottom of the water at the scene's depth (mapped), as far as the haze.
    static func floor(at z: Float) -> Mesh? {
        GroundMesh.build(segments: 64, inner: 30, outer: 20) { _, _ in (ClipSpace.map(SIMD3(0, 0, z)).z, Part.ground) }
    }

    /// A bar of `thickness` from a to b.
    static func bar(_ a: SIMD3<Float>, _ b: SIMD3<Float>, thickness t: Float) -> simd_float4x4 {
        let d = b - a
        let len = max(simd_length(d), 1e-4)
        let x = d / len
        let up: SIMD3<Float> = abs(x.z) > 0.9 ? SIMD3(1, 0, 0) : SIMD3(0, 0, 1)
        let y = simd_normalize(simd_cross(up, x)), z = simd_cross(x, y)
        return simd_float4x4(columns: (SIMD4(x * len, 0), SIMD4(y * t, 0), SIMD4(z * t, 0), SIMD4((a + b) / 2, 1)))
    }

    static func ball(_ p: SIMD3<Float>, radius r: Float) -> simd_float4x4 {
        simd_float4x4(columns: (SIMD4(r, 0, 0, 0), SIMD4(0, r, 0, 0), SIMD4(0, 0, r, 0), SIMD4(p, 1)))
    }

    /// Drawn size of a sound by what it is.
    static func radius(_ c: ClipInfo) -> Float {
        switch c.cls {
        case .bed: return 0.42
        case .texture: return 0.34
        case .creature: return 0.28
        case .event: return 0.24
        }
    }
}

// MARK: - Models of the sounds

/// What a clip is drawn as, decided from its files and sound model (the scene's JSON; `ClipInfo`
/// only keeps a Russian name, which does not tell an asteroid from a satellite).
enum ClipKind: Equatable {
    case whale(length: Float), dolphin, bubbles
    /// Drift clips with procedural water: rings of streaks around the way, going past.
    case tunnel
    case asteroid, satellite, comet, beam(length: Float), shimmer, pulsar
    /// An orbit: a moon on a faint ring (centre, radius, height of the ring).
    case moon(centre: SIMD3<Float>, radius: Float, height: Float)
    /// A sweep: a thin arc across the way.
    case sweep
    /// A lighthouse lens turning with the sound that orbits it (the orbit's centre).
    case lens(centre: SIMD3<Float>)
    /// One of the scenes' things (`Prop`, ClipObjects.swift).
    case prop(Prop)
    /// A bed that is not an object (a hull's hum, a room tone): only a small marker to point at.
    case bed
    /// Nothing known: a stone ball.
    case generic
}

/// Which model a clip gets: the first rule whose every given field matches. `files` match when
/// a word starts one of the clip's file names or one of their `_`-separated parts ("pump" in
/// "distant_pump_1.wav", not "owl" in "bowl_strike.wav"); `group`, `model` (the procedural
/// sound model) and `motion` (its type) must be equal.
struct ClipRule {
    var group: String?
    var files: [String] = []
    var model: String?
    var motion: String?
    var kind: ClipKind

    init(group: String? = nil, files: [String] = [], model: String? = nil, motion: String? = nil, _ kind: ClipKind) {
        (self.group, self.files, self.model, self.motion, self.kind) = (group, files, model, motion, kind)
    }

    func matches(group g: String, files names: [String], model m: String, motion t: String) -> Bool {
        (group == nil || group == g) && (model == nil || model == m) && (motion == nil || motion == t)
            && (files.isEmpty || names.contains { n in files.contains { n.hasPrefix($0) || n.contains("_" + $0) } })
    }

    static let all: [ClipRule] = [
        // Under water
        ClipRule(files: ["dolphin"], .dolphin),
        ClipRule(files: ["blue_whale"], .whale(length: 5.4)),
        ClipRule(files: ["fin_whale"], .whale(length: 5.0)),
        ClipRule(files: ["sperm", "humpback", "whale"], .whale(length: 4.4)),
        ClipRule(files: ["bubbles"], .bubbles),
        ClipRule(files: ["fish_grunt"], .prop(.fish)),
        ClipRule(files: ["reef_chorus"], .prop(.coral)),
        ClipRule(group: "reef", .prop(.reef)),
        ClipRule(files: ["rain_on_surface"], .prop(.rainSurface)),
        // Space
        ClipRule(files: ["freighter"], .prop(.freighter)),
        ClipRule(files: ["medium_ship"], .prop(.mediumShip)),
        ClipRule(files: ["shuttle"], .prop(.shuttle)),
        ClipRule(files: ["convoy"], .prop(.convoy)),
        ClipRule(files: ["hull_hum"], .prop(.station)),
        ClipRule(files: ["life_support", "fans"], .prop(.fan)),
        ClipRule(files: ["pump"], .prop(.pump)),
        ClipRule(files: ["hull_creak", "hull_tick", "panel_creak"], .prop(.panel)),
        ClipRule(files: ["rcs"], .prop(.rcs)),
        ClipRule(files: ["clamp"], .prop(.clamp)),
        ClipRule(files: ["airlock", "hatch"], .prop(.hatch)),
        ClipRule(files: ["robot_arm"], .prop(.robotArm)),
        ClipRule(files: ["radio"], .prop(.dish)),
        ClipRule(files: ["console", "beep"], .prop(.console)),
        ClipRule(group: "plasma", motion: "fixed", .prop(.plasma)),
        ClipRule(files: ["asteroid"], .asteroid),
        ClipRule(files: ["satellite", "juno"], .satellite),
        ClipRule(files: ["comet"], .comet),
        ClipRule(files: ["energy_beam"], .beam(length: 2.6)),
        ClipRule(files: ["ray_glide"], .beam(length: 3.6)),
        ClipRule(files: ["pulsar"], .pulsar),
        ClipRule(files: ["shimmer"], .shimmer),
        // Lighthouse
        ClipRule(files: ["optic"], .lens(centre: .zero)),
        ClipRule(files: ["clockwork"], .prop(.clockwork)),
        ClipRule(files: ["rain_on_glass", "rain_window"], .prop(.rainGlass)),
        ClipRule(files: ["surf"], .prop(.surf)),
        ClipRule(files: ["bell_buoy"], .prop(.buoy)),
        ClipRule(files: ["foghorn"], .prop(.foghorn)),
        ClipRule(files: ["pane"], .prop(.pane)),
        ClipRule(files: ["stair"], .prop(.stairs)),
        ClipRule(model: "thunder", .prop(.bolt)),
        // Night train
        ClipRule(files: ["rolling_noise"], .prop(.track)),
        ClipRule(files: ["bogie"], .prop(.bogie)),
        ClipRule(files: ["crossing"], .prop(.crossing)),
        ClipRule(files: ["passing_train"], .prop(.train)),
        ClipRule(files: ["bridge"], .prop(.bridge)),
        ClipRule(files: ["tunnel"], .prop(.arches)),
        ClipRule(files: ["tea_glass"], .prop(.teaGlass)),
        ClipRule(files: ["vent"], .prop(.vent)),
        ClipRule(files: ["horn"], .prop(.horn)),
        ClipRule(files: ["station_pass"], .prop(.lampPosts)),
        // Singing ice
        ClipRule(files: ["ice_sing", "ice_crack"], .prop(.iceStar)),
        ClipRule(files: ["ice_boom"], .prop(.boom)),
        ClipRule(files: ["ice_run"], .prop(.zigzag)),
        ClipRule(files: ["owl"], .prop(.owl)),
        ClipRule(files: ["skater"], .prop(.skater)),
        ClipRule(group: "shore", .prop(.treeLine)),
        // Tea house
        ClipRule(files: ["kettle"], .prop(.kettle)),
        ClipRule(group: "kettle", .prop(.brazier)),
        ClipRule(files: ["fukusa"], .prop(.cloth)),
        ClipRule(files: ["ladle"], .prop(.ladle)),
        ClipRule(files: ["whisk"], .prop(.whisk)),
        ClipRule(files: ["tatami"], .prop(.host)),
        ClipRule(files: ["bowl_slide"], .prop(.teaBowl)),
        ClipRule(group: "eaves", .prop(.eaves)),
        ClipRule(files: ["suikinkutsu"], .prop(.basin)),
        ClipRule(files: ["shishi"], .prop(.shishi)),
        ClipRule(files: ["furin"], .prop(.chime)),
        ClipRule(files: ["temple_bell"], .prop(.templeBell)),
        // Body scan
        ClipRule(group: "guide", .prop(.bead)),
        ClipRule(files: ["bowl_strike"], .prop(.singingBowl)),
        ClipRule(files: ["tingsha"], .prop(.tingsha)),
        // Campfire by the sea
        ClipRule(files: ["campfire_crackle_ccby"], .prop(.campfire)),
        ClipRule(files: ["campfire_crackle"], .prop(.sparks)),
        ClipRule(files: ["campfire_flare", "campfire_bonfire"], .prop(.flames)),
        ClipRule(group: "embers", model: "fire", .prop(.embers)),
        ClipRule(files: ["campfire_log"], .prop(.driftLog)),
        ClipRule(files: ["campfire_damp"], .prop(.steam)),
        ClipRule(files: ["shore_breakers"], .prop(.sea)),
        ClipRule(files: ["shore_wave"], .prop(.surf)),
        ClipRule(files: ["shore_curlew"], .prop(.curlew)),
        ClipRule(group: "grass", model: "foliage", .prop(.duneGrass)),
        // Weather from the procedural models, anywhere.
        ClipRule(model: "wind", .prop(.wind)),
        ClipRule(model: "rain", .prop(.rain)),
        ClipRule(group: "sea", model: "water", .prop(.sea)),
    ]
}

/// One mesh at a place; the renderer adds the id and the selection flags.
struct ClipPiece {
    var mesh: Mesh
    var model: simd_float4x4
    /// How see-through: 0 a solid thing, `faint` or `veryFaint` for an effect (wind, rain, a
    /// glow), drawn as a screen door that casts no shadow and draws no contour.
    var ghost: Float = 0

    static let faint: Float = 0.5, veryFaint: Float = 0.75
}

/// Building blocks for the meshes: smooth lathes and flat extrusions in the forest's part scheme.
struct MeshBuilder {
    var verts: [MeshVertex] = []
    var idx: [UInt32] = []

    /// Triangles with normals averaged over shared vertices.
    mutating func smooth(_ pos: [SIMD3<Float>], _ tri: [UInt32], part: Float) {
        var n = [SIMD3<Float>](repeating: .zero, count: pos.count)
        for t in stride(from: 0, to: tri.count, by: 3) {
            let a = Int(tri[t]), b = Int(tri[t + 1]), c = Int(tri[t + 2])
            let f = simd_cross(pos[b] - pos[a], pos[c] - pos[a])
            n[a] += f; n[b] += f; n[c] += f
        }
        let base = UInt32(verts.count)
        for (i, p) in pos.enumerated() {
            let l = simd_length(n[i])
            verts.append(MeshVertex(p, l > 1e-9 ? n[i] / l : SIMD3(0, 0, 1), part))
        }
        idx += tri.map { $0 + base }
    }

    /// Flat-shaded triangles.
    mutating func flat(_ tris: [(SIMD3<Float>, SIMD3<Float>, SIMD3<Float>)], part: Float) {
        for (a, b, c) in tris {
            var n = simd_cross(b - a, c - a)
            let l = simd_length(n)
            n = l > 1e-9 ? n / l : SIMD3(0, 0, 1)
            let base = UInt32(verts.count)
            verts += [MeshVertex(a, n, part), MeshVertex(b, n, part), MeshVertex(c, n, part)]
            idx += [base, base + 1, base + 2]
        }
    }

    /// A body of elliptical sections along x: (x, half width, half height, centre height).
    mutating func lathe(_ st: [(Float, Float, Float, Float)], segs: Int, part: Float) {
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        for s in st {
            for k in 0..<segs {
                let a = 2 * Float.pi * Float(k) / Float(segs)
                pos.append(SIMD3(s.0, s.1 * cos(a), s.3 + s.2 * sin(a)))
            }
        }
        for i in 0..<(st.count - 1) {
            for k in 0..<segs {
                let a = UInt32(i * segs + k), b = UInt32(i * segs + (k + 1) % segs)
                let c = a + UInt32(segs), d = b + UInt32(segs)
                tri += [a, c, b, b, c, d]
            }
        }
        smooth(pos, tri, part: part)
    }

    /// A flat plate: a star-shaped outline in the plane of (u, v), `t` thick along w.
    mutating func plate(_ poly: [SIMD2<Float>], thickness t: Float, origin o: SIMD3<Float> = .zero,
                        u: SIMD3<Float>, v: SIMD3<Float>, part: Float) {
        let w = simd_normalize(simd_cross(u, v))
        let c2 = poly.reduce(SIMD2<Float>.zero, +) / Float(poly.count)
        func at(_ p: SIMD2<Float>, _ s: Float) -> SIMD3<Float> { o + u * p.x + v * p.y + w * (s * t / 2) }
        var tris: [(SIMD3<Float>, SIMD3<Float>, SIMD3<Float>)] = []
        for k in 0..<poly.count {
            let p = poly[k], q = poly[(k + 1) % poly.count]
            tris.append((at(c2, 1), at(p, 1), at(q, 1)))
            tris.append((at(c2, -1), at(q, -1), at(p, -1)))
            tris.append((at(p, -1), at(q, -1), at(q, 1)))
            tris.append((at(p, -1), at(q, 1), at(p, 1)))
        }
        flat(tris, part: part)
    }

    /// A low-poly rock: an icosphere (80 faces) pushed in and out, flat shaded.
    mutating func rock(seed: UInt64, bumps: Float, squash: SIMD3<Float>, part: Float) {
        let t: Float = (1 + Float(5).squareRoot()) / 2
        var p: [SIMD3<Float>] = [
            SIMD3(-1, t, 0), SIMD3(1, t, 0), SIMD3(-1, -t, 0), SIMD3(1, -t, 0), SIMD3(0, -1, t), SIMD3(0, 1, t),
            SIMD3(0, -1, -t), SIMD3(0, 1, -t), SIMD3(t, 0, -1), SIMD3(t, 0, 1), SIMD3(-t, 0, -1), SIMD3(-t, 0, 1),
        ].map { simd_normalize($0) }
        var faces: [(Int, Int, Int)] = [
            (0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
            (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1),
        ]
        var cache: [Int: Int] = [:]
        func mid(_ a: Int, _ b: Int) -> Int {
            let key = min(a, b) * 1000 + max(a, b)
            if let m = cache[key] { return m }
            p.append(simd_normalize(p[a] + p[b]))
            cache[key] = p.count - 1
            return p.count - 1
        }
        var next: [(Int, Int, Int)] = []
        for f in faces {
            let ab = mid(f.0, f.1), bc = mid(f.1, f.2), ca = mid(f.2, f.0)
            next += [(f.0, ab, ca), (f.1, bc, ab), (f.2, ca, bc), (ab, bc, ca)]
        }
        faces = next
        var rng = SeededGenerator(seed)
        let jitter = p.map { _ in rng.next() }
        let s1 = Float(seed % 7), s2 = Float(seed % 5)
        // Split up: as one expression this takes the type checker too long on CI.
        let q = p.enumerated().map { i, d -> SIMD3<Float> in
            let a: Float = sin(2.3 * d.x + s1) * sin(2.9 * d.y + s2)
            let b: Float = 0.6 * sin(4.1 * d.z + s1) * sin(3.3 * d.x + s2)
            let k: Float = 1 + bumps * (0.6 * (a + b) + 0.8 * (jitter[i] - 0.5))
            return d * k * squash
        }
        flat(faces.map { (q[$0.0], q[$0.1], q[$0.2]) }, part: part)
    }

    func build() -> Mesh? { Mesh.make(verts, idx) }
}

/// 4x4 helpers for placing parts.
enum M4 {
    static func move(_ t: SIMD3<Float>) -> simd_float4x4 {
        simd_float4x4(columns: (SIMD4(1, 0, 0, 0), SIMD4(0, 1, 0, 0), SIMD4(0, 0, 1, 0), SIMD4(t, 1)))
    }
    static func scale(_ s: SIMD3<Float>) -> simd_float4x4 { simd_float4x4(diagonal: SIMD4(s, 1)) }
    static func scale(_ s: Float) -> simd_float4x4 { scale(SIMD3(repeating: s)) }
    static func rotX(_ a: Float) -> simd_float4x4 {
        let c = cos(a), s = sin(a)
        return simd_float4x4(columns: (SIMD4(1, 0, 0, 0), SIMD4(0, c, s, 0), SIMD4(0, -s, c, 0), SIMD4(0, 0, 0, 1)))
    }
    static func rotY(_ a: Float) -> simd_float4x4 {
        let c = cos(a), s = sin(a)
        return simd_float4x4(columns: (SIMD4(c, 0, -s, 0), SIMD4(0, 1, 0, 0), SIMD4(s, 0, c, 0), SIMD4(0, 0, 0, 1)))
    }
    static func rotZ(_ a: Float) -> simd_float4x4 {
        let c = cos(a), s = sin(a)
        return simd_float4x4(columns: (SIMD4(c, s, 0, 0), SIMD4(-s, c, 0, 0), SIMD4(0, 0, 1, 0), SIMD4(0, 0, 0, 1)))
    }
    /// Local x along `f`, z up as far as it can be.
    static func facing(_ f: SIMD3<Float>) -> simd_float4x4 {
        let x = simd_normalize(f)
        let up: SIMD3<Float> = abs(x.z) > 0.95 ? SIMD3(1, 0, 0) : SIMD3(0, 0, 1)
        let y = simd_normalize(simd_cross(up, x)), z = simd_cross(x, y)
        return simd_float4x4(columns: (SIMD4(x, 0), SIMD4(y, 0), SIMD4(z, 0), SIMD4(0, 0, 0, 1)))
    }
}

/// The meshes of the models, built once.
enum ClipModels {
    private static let flukePoly: [SIMD2<Float>] = {
        let half: [SIMD2<Float>] = [SIMD2(-0.86, 0.03), SIMD2(-0.97, 0.14), SIMD2(-1.10, 0.36), SIMD2(-0.98, 0.31), SIMD2(-0.93, 0.10)]
        return half + [SIMD2(-0.94, 0)] + half.reversed().map { SIMD2($0.x, -$0.y) }
    }()

    /// Whale: length 2 along +x, the nose at +1. The body ends at the joint x = -0.3.
    static let whaleBody: Mesh? = {
        var b = MeshBuilder()
        b.lathe([(1.0, 0.02, 0.02, -0.01), (0.94, 0.10, 0.09, -0.01), (0.78, 0.17, 0.15, 0.0), (0.42, 0.22, 0.20, 0.01),
                 (0.0, 0.23, 0.22, 0.02), (-0.32, 0.18, 0.17, 0.03)], segs: 10, part: Part.stone)
        b.plate([SIMD2(-0.12, 0.2), SIMD2(-0.2, 0.31), SIMD2(-0.3, 0.2)], thickness: 0.03,
                u: SIMD3(1, 0, 0), v: SIMD3(0, 0, 1), part: Part.stone)
        return b.build()
    }()

    /// Whale tail: from the joint at x = -0.3 to the flukes; turns about that joint.
    static let whaleTail: Mesh? = {
        var b = MeshBuilder()
        b.lathe([(-0.30, 0.18, 0.17, 0.03), (-0.60, 0.11, 0.11, 0.04), (-0.85, 0.05, 0.07, 0.05), (-0.96, 0.025, 0.04, 0.05)],
                segs: 10, part: Part.stone)
        b.plate(flukePoly, thickness: 0.025, origin: SIMD3(0, 0, 0.05), u: SIMD3(1, 0, 0), v: SIMD3(0, 1, 0), part: Part.stone)
        return b.build()
    }()

    /// Dolphin: length 2, beak forward, a tall dorsal fin; the joint at x = -0.45.
    static let dolphinBody: Mesh? = {
        var b = MeshBuilder()
        b.lathe([(1.0, 0.012, 0.012, -0.02), (0.92, 0.04, 0.035, -0.03), (0.76, 0.055, 0.05, -0.03), (0.62, 0.11, 0.12, 0),
                 (0.38, 0.19, 0.21, 0.02), (0.0, 0.23, 0.25, 0.03), (-0.45, 0.16, 0.18, 0.04)], segs: 10, part: Part.stone)
        b.plate([SIMD2(0.14, 0.23), SIMD2(0.0, 0.46), SIMD2(-0.14, 0.52), SIMD2(-0.12, 0.38), SIMD2(-0.3, 0.22)], thickness: 0.035,
                u: SIMD3(1, 0, 0), v: SIMD3(0, 0, 1), part: Part.stone)
        return b.build()
    }()

    static let dolphinTail: Mesh? = {
        var b = MeshBuilder()
        b.lathe([(-0.45, 0.16, 0.18, 0.04), (-0.72, 0.09, 0.11, 0.05), (-0.88, 0.04, 0.06, 0.05), (-0.96, 0.02, 0.035, 0.05)],
                segs: 10, part: Part.stone)
        b.plate(flukePoly, thickness: 0.02, origin: SIMD3(0, 0, 0.05), u: SIMD3(1, 0, 0), v: SIMD3(0, 1, 0), part: Part.stone)
        return b.build()
    }()

    /// A flipper of the left side (+y), reaching out and back.
    static let flipper: Mesh? = {
        var b = MeshBuilder()
        b.plate([SIMD2(0.09, 0), SIMD2(-0.09, 0), SIMD2(-0.22, 0.30), SIMD2(-0.14, 0.34), SIMD2(-0.02, 0.15)], thickness: 0.025,
                u: SIMD3(1, 0, 0), v: SIMD3(0, 1, 0), part: Part.stone)
        return b.build()
    }()

    static let asteroids: [Mesh] = [(UInt64(11), SIMD3<Float>(1.0, 0.8, 0.7)), (UInt64(23), SIMD3(0.85, 1.0, 0.75)), (UInt64(37), SIMD3(1.0, 0.9, 0.6))].compactMap {
        var b = MeshBuilder()
        b.rock(seed: $0.0, bumps: 0.34, squash: $0.1, part: Part.stone)
        return b.build()
    }

    static let moon: Mesh? = {
        var b = MeshBuilder()
        b.rock(seed: 5, bumps: 0.10, squash: SIMD3(repeating: 1), part: Part.stone)
        return b.build()
    }()

    static let comet: Mesh? = {
        var b = MeshBuilder()
        b.rock(seed: 3, bumps: 0.22, squash: SIMD3(1.1, 0.9, 0.9), part: Part.stone)
        return b.build()
    }()

    /// A four-pointed star in two crossed plates.
    static let star: Mesh? = {
        var b = MeshBuilder()
        let poly: [SIMD2<Float>] = [SIMD2(0, 1), SIMD2(0.16, 0.16), SIMD2(1, 0), SIMD2(0.16, -0.16),
                                    SIMD2(0, -1), SIMD2(-0.16, -0.16), SIMD2(-1, 0), SIMD2(-0.16, 0.16)]
        b.plate(poly, thickness: 0.06, u: SIMD3(1, 0, 0), v: SIMD3(0, 1, 0), part: Part.stone)
        b.plate(poly, thickness: 0.06, u: SIMD3(1, 0, 0), v: SIMD3(0, 0, 1), part: Part.stone)
        return b.build()
    }()
}

/// Decides each clip's model and poses it frame by frame.
final class ClipActors {
    private var heading: [UInt32: SIMD3<Float>] = [:]
    private var kinds: [UInt32: ClipKind] = [:]
    private var kindsFor = ""
    /// Meshes built in place for one clip (a track, a bridge, a surface), mapped like the stage.
    var placed: [UInt32: Mesh] = [:]
    /// The scene's own still things (the ice, the seated listener), for this scene.
    var scenery: [ClipPiece] = []

    // MARK: Which model

    /// Reads the files and sound model of every clip from the scene's JSON.
    func kind(of id: UInt32, in scene: SceneInfo) -> ClipKind {
        let key = scene.entry.id + "#" + String(scene.json?.utf8.count ?? 0)
        if key != kindsFor {
            kindsFor = key
            kinds = Self.classify(scene)
            heading.removeAll()
            placed.removeAll()
            scenery = Self.scenery(kinds: Array(kinds.values), scene: scene)
        }
        return kinds[id] ?? .generic
    }

    private static func classify(_ scene: SceneInfo) -> [UInt32: ClipKind] {
        guard let json = scene.json,
              let root = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
              let entities = root["entities"] as? [[String: Any]] else { return [:] }
        var out: [UInt32: ClipKind] = [:]
        for (i, e) in entities.enumerated() where e["kind"] as? String == "clip" {
            let id = (e["id"] as? NSNumber)?.uint32Value ?? UInt32(1000 + i)
            let files = (e["files"] as? [String] ?? []).map { ($0 as NSString).lastPathComponent.lowercased() }
            let group = e["group"] as? String ?? ""
            let motion = e["motion"] as? [String: Any] ?? [:]
            let type = motion["type"] as? String ?? "fixed"
            let model = (e["sound"] as? [String: Any])?["model"] as? String ?? e["model"] as? String ?? ""
            func f(_ k: String) -> Float? { (motion[k] as? NSNumber)?.floatValue }
            let c = (motion["center"] as? [NSNumber])?.map(\.floatValue) ?? [0, 0, 0]
            let centre = c.count > 2 ? SIMD3(c[0], c[1], c[2]) : .zero
            var k: ClipKind
            if let rule = ClipRule.all.first(where: { $0.matches(group: group, files: files, model: model, motion: type) }) {
                k = rule.kind
                if case .lens = k { k = .lens(centre: centre) }
            } else if e["tunnel"] as? Bool == true { k = .tunnel }
            else if type == "sweep" { k = .sweep }
            else if type == "orbit" && !files.isEmpty { k = .moon(centre: centre, radius: f("radius") ?? 5, height: f("height") ?? 0) }
            else if files.isEmpty || e["class"] as? String ?? "bed" == "bed" { k = .bed }
            else { k = .generic }
            out[id] = k
        }
        return out
    }

    // MARK: Posing

    /// The pieces of one clip at its mapped place `p` (raw position `raw`), and the half size of the
    /// space it takes (for the selection brackets).
    func pieces(_ kind: ClipKind, clip: ClipInfo, raw: SIMD3<Float>, trail: [SIMD3<Float>], time t: Float, level: Float,
                screen: (distance: Float, width: Float)) -> (pieces: [ClipPiece], extent: Float) {
        var made = posed(kind, clip: clip, raw: raw, trail: trail, time: t, level: level, screen: screen)
        // Effects rather than things: see-through (the comet's head stays a rock).
        switch kind {
        case .tunnel, .bubbles, .beam, .shimmer, .pulsar, .sweep:
            for i in made.pieces.indices { made.pieces[i].ghost = ClipPiece.faint }
        case .comet:
            for i in made.pieces.indices.dropFirst() { made.pieces[i].ghost = ClipPiece.faint }
        default:
            break
        }
        return made
    }

    private func posed(_ kind: ClipKind, clip: ClipInfo, raw: SIMD3<Float>, trail: [SIMD3<Float>], time t: Float, level: Float,
                       screen: (distance: Float, width: Float)) -> (pieces: [ClipPiece], extent: Float) {
        guard let sphere = ClipProps.sphere, let box = ClipProps.box else { return ([], 0.4) }
        let p = ClipSpace.map(raw)
        let id = clip.id
        let ph = Float(id % 13) * 0.9
        let pulse = 1 + 0.08 * level
        var out: [ClipPiece] = []

        func fwd() -> SIMD3<Float> {
            var d = heading[id] ?? SIMD3(1, 0, 0)
            if trail.count > 3 {
                let v = p - ClipSpace.map(trail[max(0, trail.count - 5)])
                if simd_length(v) > 0.02 {
                    var n = simd_normalize(v)
                    n.z *= 0.5
                    n = simd_normalize(n)
                    d = simd_normalize(d * 0.88 + n * 0.12)
                }
            } else if clip.motion == "drift" {
                d = SIMD3(-1, 0, 0)
            }
            heading[id] = d
            return d
        }
        func ball(_ q: SIMD3<Float>, _ r: Float) { out.append(ClipPiece(mesh: sphere, model: ClipProps.ball(q, radius: r))) }
        func bar(_ a: SIMD3<Float>, _ b: SIMD3<Float>, _ th: Float) { out.append(ClipPiece(mesh: box, model: ClipProps.bar(a, b, thickness: th))) }

        switch kind {
        case .whale, .dolphin:
            var isWhale = false
            var length: Float = 1.7
            if case .whale(let l) = kind { isWhale = true; length = l }
            guard let body = isWhale ? ClipModels.whaleBody : ClipModels.dolphinBody,
                  let tail = isWhale ? ClipModels.whaleTail : ClipModels.dolphinTail,
                  let fl = ClipModels.flipper else { break }
            let len = length
            let half = len / 2
            let sway = isWhale ? 0.13 * sin(t * 1.2 + ph) : 0.26 * sin(t * 3.4 + ph)
            let roll = isWhale ? 0.04 * sin(t * 0.7 + ph) : 0.16 * sin(t * 1.1 + ph)
            let bob = (isWhale ? 0.05 : 0.04) * sin(t * (isWhale ? 1.2 : 3.4) + ph - 0.9)
            let root = M4.move(p + SIMD3(0, 0, bob)) * M4.facing(fwd()) * M4.rotX(roll) * M4.scale(half * pulse)
            out.append(ClipPiece(mesh: body, model: root))
            let jx: Float = isWhale ? -0.3 : -0.45
            out.append(ClipPiece(mesh: tail, model: root * M4.move(SIMD3(jx, 0, 0.04)) * M4.rotY(sway) * M4.move(SIMD3(-jx, 0, -0.04))))
            let fs: Float = isWhale ? 1.0 : 0.62
            let fx: Float = isWhale ? 0.4 : 0.38
            let flap = 0.12 * sin(t * (isWhale ? 1.2 : 2.4) + ph)
            let left = root * M4.move(SIMD3(fx, 0.14, -0.1)) * M4.rotX(-0.55 + flap) * M4.scale(fs)
            let right = root * M4.move(SIMD3(fx, -0.14, -0.1)) * M4.scale(SIMD3(1, -1, 1)) * M4.rotX(-0.55 + flap) * M4.scale(fs)
            out.append(ClipPiece(mesh: fl, model: left))
            out.append(ClipPiece(mesh: fl, model: right))
            return (out, half * 1.05)

        case .bubbles:
            for i in 0..<8 {
                let f = Float(i)
                let life = (t * 0.16 + f * 0.137 + ph * 0.1).truncatingRemainder(dividingBy: 1)
                let r = (0.05 + 0.045 * fract(f * 0.618)) * pulse * min(1, min(life * 8, (1 - life) * 4 + 0.3))
                let q = p + SIMD3(0.16 * sin(f * 2.1 + t * 0.9), 0.16 * cos(f * 1.7 + t * 0.8), (life - 0.35) * 1.3)
                ball(q, r)
            }
            return (out, 0.55)

        case .tunnel:
            // Rings of short streaks around the way, at the clip's place along it.
            let radius = max(clip.spread, 2.5)
            let spin = t * 0.15 + ph
            for k in 0..<5 {
                let a0 = spin + Float(k) * 2 * .pi / 5
                var prev: SIMD3<Float>?
                for s in 0...3 {
                    let a = a0 + Float(s) * 0.17
                    let q = ClipSpace.map(SIMD3(raw.x, radius * cos(a), radius * sin(a)))
                    if let pr = prev { bar(pr, q, 0.022) }
                    prev = q
                }
            }
            return (out, 0.3)

        case .asteroid:
            let meshes = ClipModels.asteroids
            guard !meshes.isEmpty else { break }
            let m = meshes[Int(id) % meshes.count]
            let r = (0.5 + 0.07 * Float(id % 3)) * pulse
            let axis = simd_normalize(SIMD3<Float>(0.6, 0.3 + 0.1 * Float(id % 3), 0.7))
            let spin = simd_quatf(angle: t * 0.5 + ph, axis: axis)
            out.append(ClipPiece(mesh: m, model: M4.move(p) * simd_float4x4(spin) * M4.scale(r)))
            return (out, r * 1.1)

        case .satellite:
            let tumble = M4.facing(fwd()) * M4.rotX(t * 0.35 + ph) * M4.rotY(0.4 * sin(t * 0.3 + ph))
            let root = M4.move(p) * tumble * M4.scale(1.0 * pulse)
            func part(_ c: SIMD3<Float>, _ s: SIMD3<Float>) { out.append(ClipPiece(mesh: box, model: root * M4.move(c) * M4.scale(s))) }
            part(SIMD3(0, 0, 0), SIMD3(0.34, 0.26, 0.26))
            for side: Float in [1, -1] {
                part(SIMD3(0, side * 0.3, 0), SIMD3(0.04, 0.34, 0.04))
                for i in 0..<3 {
                    part(SIMD3(0, side * (0.5 + 0.25 * Float(i)), 0), SIMD3(0.36, 0.22, 0.02))
                }
            }
            out.append(ClipPiece(mesh: sphere, model: root * M4.move(SIMD3(0.24, 0, 0)) * M4.scale(SIMD3(0.07, 0.16, 0.16))))
            return (out, 1.0)

        case .comet:
            // A rock head and a tail of the trail: three thin streaks that taper and part as they age.
            guard let head = ClipModels.comet else { break }
            out.append(ClipPiece(mesh: head, model: M4.move(p) * M4.rotZ(t * 0.7 + ph) * M4.scale(0.28 * pulse)))
            if trail.count > 2 {
                let pts = trail.map { ClipSpace.map($0) } + [p]
                let n = pts.count
                for lane in -1...1 {
                    for i in 0..<(n - 1) where i % 2 == 0 || lane == 0 {
                        let age = Float(i + 1) / Float(n)
                        let j = min(i + 2, n - 1)
                        let off = SIMD3<Float>(0, 0.09 * Float(lane) * (1 - age) * 3, 0.04 * Float(lane) * (1 - age) * 3)
                        bar(pts[i] + off * (1 - age), pts[j] + off * (1 - age), 0.01 + 0.075 * age * (lane == 0 ? 1 : 0.5))
                    }
                }
            }
            return (out, 0.4)

        case .beam(let length):
            let d = fwd()
            let a = p - d * length, b = p + d * 0.15
            bar(a, b, 0.035)
            bar(p - d * length * 0.35, b + d * 0.1, 0.075)
            return (out, length * 0.45)

        case .shimmer:
            if let star = ClipModels.star {
                out.append(ClipPiece(mesh: star, model: M4.move(p) * M4.rotZ(t * 0.4 + ph) * M4.rotX(t * 0.23) * M4.scale(0.4 * pulse)))
            }
            return (out, 0.35)

        case .pulsar:
            ball(p, 0.13 * pulse)
            let a = t * 1.3 + ph
            let d = SIMD3<Float>(cos(a) * 0.9, sin(a) * 0.9, 0.35)
            bar(p - d, p + d, 0.03)
            return (out, 0.9)

        case .moon(let centre, let radius, let height):
            // The moon on its path: a dashed ring of the true orbit, mapped like everything else.
            let n = 48
            let ring = (0..<n).map { ClipSpace.map(centre + SIMD3(radius * cos(2 * .pi * Float($0) / Float(n)),
                                                                radius * sin(2 * .pi * Float($0) / Float(n)), height)) }
            for i in 0..<n where i % 3 != 2 { bar(ring[i], ring[(i + 1) % n], 0.016) }
            if let m = ClipModels.moon {
                out.append(ClipPiece(mesh: m, model: M4.move(p) * M4.rotZ(t * 0.2 + ph) * M4.scale((id % 2 == 0 ? 0.36 : 0.27) * pulse)))
            }
            return (out, 0.35)

        case .sweep:
            // A thin arc across the way, bowed towards the listener, as wide as the screen where it passes it.
            let sw = clip.sweep ?? (-40, 40, 10, 0)
            let x = raw.x, d = screen.distance
            let wide = x <= d
                ? screen.width + (sw.width - screen.width) * min(max((d - x) / max(d - sw.fromX, 0.1), 0), 1)
                : screen.width + (sw.width - screen.width) * min(max((x - d) / max(sw.toX - d, 0.1), 0), 1)
            let segs = 14
            var prev: SIMD3<Float>?
            for s in 0...segs {
                let u = Float(s) / Float(segs) * 2 - 1
                let q = ClipSpace.map(SIMD3(x - 0.18 * wide * (1 - u * u), u * wide / 2, sw.height))
                if let pr = prev { bar(pr, q, 0.03) }
                prev = q
            }
            ball(ClipSpace.map(SIMD3(x, 0, sw.height)), 0.07)
            return (out, 0.4)

        case .lens(let centre):
            // The lens turns on its pedestal at the orbit's centre, its bull's-eye on the sound.
            guard let lens = PropMeshes.lens else { break }
            let c = ClipSpace.map(centre)
            let a = atan2(raw.y - centre.y, raw.x - centre.x)
            out.append(ClipPiece(mesh: lens, model: M4.move(c) * M4.rotZ(a) * M4.scale(0.75 * pulse)))
            // Its light going out over the sea where the lens faces.
            if let beam = PropMeshes.lightBeam {
                out.append(ClipPiece(mesh: beam, model: M4.move(c) * M4.rotZ(a) * M4.rotY(0.05) * M4.scale(SIMD3(7, 5, 5)),
                                     ghost: ClipPiece.veryFaint))
            }
            return (out, simd_distance(c, p) + 0.5)

        case .prop(let prop):
            let extent = propPieces(prop, clip: clip, raw: raw, time: t, level: level, fwd: fwd, out: &out)
            if !out.isEmpty { return (out, extent) }

        case .bed:
            // A hum about the head itself (a room tone, the air) is the listener's own space: no mark.
            if simd_length(raw) < 1.2 { return ([], 0.4) }
            let extent = propPieces(.marker, clip: clip, raw: raw, time: t, level: level, fwd: fwd, out: &out)
            return (out, extent)

        case .generic:
            break
        }
        ball(p, ClipProps.radius(clip) * pulse)
        return (out, ClipProps.radius(clip) + 0.12)
    }

    private func fract(_ x: Float) -> Float { x - floor(x) }
}
