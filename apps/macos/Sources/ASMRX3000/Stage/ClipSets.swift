import Foundation
import simd

/// The place a clip scene happens in, drawn around the listener so they feel placed. Only the
/// body-scan scene has one now: a dark floor with a round mat under the listener. The forest
/// and the underwater scenes draw their own looks and need no set. The place is the "set":
/// quiet linework and surfaces in their own material (`Part.set`, a pale pencil grey that
/// never takes a contour or the selection), always there, behind the sounds.
///
/// Everything goes through the stage's radial compression (`ClipSpace.map`), so a sound at the
/// glass sits on the glass; lines are built in mapped space so they keep one drawn width near
/// and far.
enum ClipSet: Equatable {
    case none, underwater, bodyScan

    static func of(_ scene: SceneInfo) -> ClipSet {
        let id = scene.entry.id
        let name = ((scene.entry.file ?? id) as NSString).lastPathComponent
        func has(_ s: String) -> Bool { id.hasPrefix(s) || name.hasPrefix(s) || id.contains("/" + s) }
        if has("underwater") { return .underwater }
        if has("body_scan") { return .bodyScan }
        switch scene.look {
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
        case .bodyScan:
            c.target = SIMD3(0.4, 0, -0.6); c.elevation = 0.5; c.distance = 7
        case .underwater:
            // Under the surface: the eye stays below it.
            c.target = SIMD3(1.2, 0, -0.6); c.elevation = 0.22; c.distance = 17
        case .none:
            c.target = SIMD3(1.2, 0, -0.4); c.elevation = 0.36; c.distance = 19
        }
        return c
    }

    /// The sky: how many stars (0...1). None of the kept places has any.
    var stars: Float { 0 }

    /// Whether a planet fills the lower sky (no kept place has one).
    var planet: Bool { false }

    /// Where a sound is drawn: where it sounds.
    func drawnAt(_ kind: ClipKind, raw: SIMD3<Float>) -> SIMD3<Float> { raw }

    // MARK: Geometry

    /// The still part of the place, built once per scene.
    func scenery(groundZ: Float) -> [ClipPiece] {
        var k = SetKit()
        var ground = SetKit()
        switch self {
        case .bodyScan: Self.bodyScan(&k, &ground, z: groundZ)
        case .underwater, .none: break
        }
        var out: [ClipPiece] = []
        for kit in [ground, k] {
            if !kit.b.verts.isEmpty, let m = kit.b.build() { out.append(ClipPiece(mesh: m, model: matrix_identity_float4x4)) }
        }
        return out
    }

    /// What moves with the place, posed for time `t` (nothing in the kept places).
    func moving(time t: Float) -> [ClipPiece] { [] }

    // MARK: Places

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
