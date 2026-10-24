import simd

/// Primitives for the clip scenes' objects, on top of `MeshBuilder`: boxes, rods, turned
/// (revolved) bodies, hoops and tubes along a path, each placed by a matrix, so a model is a
/// short list of parts and ends up as one mesh (one draw call).
extension MeshBuilder {
    /// Appends `o` moved by `m` (normals by its inverse transpose), optionally in another part.
    mutating func add(_ o: MeshBuilder, _ m: simd_float4x4, part: Float? = nil) {
        let base = UInt32(verts.count)
        let m3 = simd_float3x3(SIMD3(m.columns.0.x, m.columns.0.y, m.columns.0.z),
                               SIMD3(m.columns.1.x, m.columns.1.y, m.columns.1.z),
                               SIMD3(m.columns.2.x, m.columns.2.y, m.columns.2.z))
        let nm = m3.inverse.transpose
        for v in o.verts {
            let p = m * SIMD4(v.position, 1)
            var n = nm * SIMD3(v.nx, v.ny, v.nz)
            let l = simd_length(n)
            n = l > 1e-9 ? n / l : SIMD3(0, 0, 1)
            verts.append(MeshVertex(SIMD3(p.x, p.y, p.z), n, part ?? v.part))
        }
        idx += o.idx.map { $0 + base }
    }

    /// The same triangles with every vertex taken through `f` (the stage's radial compression),
    /// normals recomputed: shared vertices stay smooth, split ones flat.
    func warped(_ f: (SIMD3<Float>) -> SIMD3<Float>) -> MeshBuilder {
        let pos = verts.map { f($0.position) }
        var n = [SIMD3<Float>](repeating: .zero, count: pos.count)
        for t in stride(from: 0, to: idx.count, by: 3) {
            let a = Int(idx[t]), b = Int(idx[t + 1]), c = Int(idx[t + 2])
            let fn = simd_cross(pos[b] - pos[a], pos[c] - pos[a])
            n[a] += fn; n[b] += fn; n[c] += fn
        }
        var out = MeshBuilder()
        out.verts = pos.indices.map { i in
            let l = simd_length(n[i])
            return MeshVertex(pos[i], l > 1e-12 ? n[i] / l : SIMD3(0, 0, 1), verts[i].part)
        }
        out.idx = idx
        return out
    }

    /// A box of `size` centred on `c`, turned by `rot`.
    mutating func box(_ c: SIMD3<Float>, _ size: SIMD3<Float>, part: Float, rot: simd_float4x4 = matrix_identity_float4x4) {
        add(Prim.box, M4.move(c) * rot * M4.scale(size), part: part)
    }

    /// A rod from a to b, radius ra at a and rb at b (a cone when one is 0), capped.
    mutating func rod(_ a: SIMD3<Float>, _ b: SIMD3<Float>, _ ra: Float, _ rb: Float? = nil, segs: Int = 6, part: Float) {
        let d = b - a
        let len = simd_length(d)
        guard len > 1e-6 else { return }
        let r2 = rb ?? ra
        let z = d / len
        let up: SIMD3<Float> = abs(z.z) > 0.9 ? SIMD3(1, 0, 0) : SIMD3(0, 0, 1)
        let x = simd_normalize(simd_cross(up, z)), y = simd_cross(z, x)
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        var ringA: [SIMD3<Float>] = [], ringB: [SIMD3<Float>] = []
        for k in 0..<segs {
            let t = 2 * Float.pi * Float(k) / Float(segs)
            let o = x * cos(t) + y * sin(t)
            ringA.append(a + o * ra)
            ringB.append(b + o * r2)
            pos.append(a + o * ra)
            pos.append(b + o * r2)
        }
        for k in 0..<segs {
            let i = UInt32(2 * k), j = UInt32(2 * ((k + 1) % segs))
            tri += [i, j, i + 1, j, j + 1, i + 1]
        }
        smooth(pos, tri, part: part)
        var caps: [(SIMD3<Float>, SIMD3<Float>, SIMD3<Float>)] = []
        for k in 1..<(segs - 1) {
            if ra > 1e-5 { caps.append((ringA[0], ringA[k + 1], ringA[k])) }
            if r2 > 1e-5 { caps.append((ringB[0], ringB[k], ringB[k + 1])) }
        }
        flat(caps, part: part)
    }

    /// Rods along a path (a crack, a field line, a handrail), radius tapering from r0 to r1.
    mutating func path(_ pts: [SIMD3<Float>], _ r0: Float, _ r1: Float? = nil, segs: Int = 5, part: Float) {
        guard pts.count > 1 else { return }
        let n = Float(pts.count - 1)
        for i in 0..<(pts.count - 1) {
            let ra = r0 + ((r1 ?? r0) - r0) * Float(i) / n
            let rb = r0 + ((r1 ?? r0) - r0) * Float(i + 1) / n
            rod(pts[i], pts[i + 1], ra, rb, segs: segs, part: part)
        }
    }

    /// A body turned about z from a profile of (radius, height), placed by `m`.
    mutating func turn(_ profile: [SIMD2<Float>], segs: Int = 12, part: Float, at m: simd_float4x4 = matrix_identity_float4x4) {
        add(Prim.turned(profile, segs: segs, part: part), m)
    }

    /// A ball (low poly).
    mutating func ball(_ c: SIMD3<Float>, _ r: Float, part: Float, squash: SIMD3<Float> = SIMD3(1, 1, 1)) {
        add(Prim.ball, M4.move(c) * M4.scale(squash * r), part: part)
    }

    /// A hoop of radius R and thickness r about z (from angle a0 to a1), placed by `m`.
    mutating func hoop(_ R: Float, _ r: Float, segs: Int = 20, from a0: Float = 0, to a1: Float = 2 * .pi, part: Float,
                       at m: simd_float4x4 = matrix_identity_float4x4) {
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        let sides = 5
        let closed = abs(a1 - a0 - 2 * .pi) < 1e-3
        let n = closed ? segs : segs + 1
        for i in 0..<n {
            let a = a0 + (a1 - a0) * Float(i) / Float(segs)
            let c = SIMD3(cos(a), sin(a), Float(0))
            for k in 0..<sides {
                let b = 2 * Float.pi * Float(k) / Float(sides)
                pos.append(c * (R + r * cos(b)) + SIMD3(0, 0, r * sin(b)))
            }
        }
        for i in 0..<(closed ? n : n - 1) {
            let i2 = (i + 1) % n
            for k in 0..<sides {
                let k2 = (k + 1) % sides
                let p = UInt32(i * sides + k), q = UInt32(i * sides + k2)
                let r2 = UInt32(i2 * sides + k), s = UInt32(i2 * sides + k2)
                tri += [p, r2, q, q, r2, s]
            }
        }
        var b = MeshBuilder()
        b.smooth(pos, tri, part: part)
        add(b, m)
    }

    /// A flat plate in the plane of (u, v) at `o`, from an outline star-shaped about its centre.
    mutating func sheet(_ poly: [SIMD2<Float>], _ t: Float, at o: SIMD3<Float>, u: SIMD3<Float>, v: SIMD3<Float>, part: Float) {
        plate(poly, thickness: t, origin: o, u: u, v: v, part: part)
    }
}

/// Unit primitives, built once.
enum Prim {
    /// A unit cube (-0.5...0.5), flat shaded.
    static let box: MeshBuilder = {
        var b = MeshBuilder()
        var tris: [(SIMD3<Float>, SIMD3<Float>, SIMD3<Float>)] = []
        let axes: [SIMD3<Float>] = [SIMD3(1, 0, 0), SIMD3(0, 1, 0), SIMD3(0, 0, 1)]
        for a in axes {
            for sign: Float in [1, -1] {
                let n = a * sign
                let u = SIMD3(a.y, a.z, a.x), w = simd_cross(n, u)
                let c = [(-1, -1), (1, -1), (1, 1), (-1, 1)].map { (i: Float, j: Float) in 0.5 * (n + u * i + w * j) }
                tris.append((c[0], c[1], c[2]))
                tris.append((c[0], c[2], c[3]))
            }
        }
        b.flat(tris, part: Part.stone)
        return b
    }()

    /// A unit ball, 8 x 6.
    static let ball: MeshBuilder = {
        var prof: [SIMD2<Float>] = []
        for i in 0...6 {
            let a = -Float.pi / 2 + Float.pi * Float(i) / 6
            prof.append(SIMD2(cos(a), sin(a)))
        }
        return turned(prof, segs: 8, part: Part.stone)
    }()

    /// A body turned about z: rings at each (radius, height), smooth.
    static func turned(_ prof: [SIMD2<Float>], segs: Int, part: Float) -> MeshBuilder {
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        for p in prof {
            for k in 0..<segs {
                let a = 2 * Float.pi * Float(k) / Float(segs)
                pos.append(SIMD3(p.x * cos(a), p.x * sin(a), p.y))
            }
        }
        for i in 0..<(prof.count - 1) {
            for k in 0..<segs {
                let a = UInt32(i * segs + k), b = UInt32(i * segs + (k + 1) % segs)
                let c = a + UInt32(segs), d = b + UInt32(segs)
                tri += [a, b, c, b, d, c]
            }
        }
        var m = MeshBuilder()
        m.smooth(pos, tri, part: part)
        return m
    }
}

extension M4 {
    /// Local z along `d`.
    static func along(_ d: SIMD3<Float>) -> simd_float4x4 {
        let z = simd_normalize(d)
        let up: SIMD3<Float> = abs(z.z) > 0.95 ? SIMD3(1, 0, 0) : SIMD3(0, 0, 1)
        let x = simd_normalize(simd_cross(up, z)), y = simd_cross(z, x)
        return simd_float4x4(columns: (SIMD4(x, 0), SIMD4(y, 0), SIMD4(z, 0), SIMD4(0, 0, 0, 1)))
    }

    /// The turn `r` about the point `c`.
    static func about(_ c: SIMD3<Float>, _ r: simd_float4x4) -> simd_float4x4 { move(c) * r * move(-c) }
}
