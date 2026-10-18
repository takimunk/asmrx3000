import CAsmr
import Foundation
import simd

/// The relief of flight mode for the picture: the same heightfield the engine hears
/// (`asmr_terrain_sample`: hills, mountains, dunes, coast, valleys, lakes, rivers, the sea),
/// in world-anchored chunks so nothing swims as the listener flies. Near chunks (40 m, 2.5 m
/// grid) and far chunks (160 m, 10 m grid, to the haze) are each sampled and meshed once, then
/// drawn with a model matrix that puts them in the travel frame:
///   travel = R(−heading) · (world − listener),  z = height − ears.
/// Water bodies get their own flat meshes at the water level, carrying depth (shore foam),
/// flow (rivers) and kind (lake, river, sea) for the water shader.
final class FlightTerrain {
    struct Level {
        let size: Double
        let step: Double
        let radius: Double
        /// Far chunks sit a little lower so the near ones cover them where both exist.
        let sink: Float
    }
    static let levels = [Level(size: 40, step: 2.5, radius: 115, sink: 0), Level(size: 160, step: 10, radius: 560, sink: 0.35)]

    struct Key: Hashable { var level: Int; var i: Int; var j: Int }

    final class Chunk {
        let key: Key
        let origin: SIMD2<Double>
        let n: Int
        let step: Float
        /// Ground heights (absolute), n × n, index j·n + i.
        let ground: [Float]
        let water: [Float]
        let groundMesh: Mesh?
        let waterMesh: Mesh?

        init(key: Key, origin: SIMD2<Double>, n: Int, step: Float, ground: [Float], water: [Float], groundMesh: Mesh?, waterMesh: Mesh?) {
            self.key = key
            self.origin = origin
            self.n = n
            self.step = step
            self.ground = ground
            self.water = water
            self.groundMesh = groundMesh
            self.waterMesh = waterMesh
        }
    }

    /// Chunks computed per frame (more right after the landscape changed); `.max` in snapshots.
    static var budget = 3

    private var sampler: OpaquePointer?
    private var landKey: (UInt64, UInt32, Float, UInt32)?
    private(set) var chunks: [Key: Chunk] = [:]
    private(set) var frame = AsmrFlightFrame()
    private var buffer: [Float] = []
    private var fresh = 0

    deinit {
        if let sampler { asmr_terrain_free(sampler) }
    }

    /// Follows the engine's landscape and the listener; computes missing chunks nearest first.
    func update(_ f: AsmrFlightFrame) {
        frame = f
        let key = (f.seed, f.biome, f.relief, f.generation)
        if landKey == nil || landKey! != key {
            landKey = key
            if let sampler { asmr_terrain_free(sampler) }
            sampler = asmr_terrain_new(f.seed, f.biome, f.relief)
            chunks.removeAll()
            fresh = 30
        }
        var missing: [(Double, Key)] = []
        var wanted = Set<Key>()
        for (li, lv) in Self.levels.enumerated() {
            let k = Int((lv.radius / lv.size).rounded(.up)) + 1
            let ci = Int((f.x / lv.size).rounded(.down)), cj = Int((f.y / lv.size).rounded(.down))
            for a in -k...k {
                for b in -k...k {
                    let key = Key(level: li, i: ci + a, j: cj + b)
                    let cx = (Double(key.i) + 0.5) * lv.size - f.x, cy = (Double(key.j) + 0.5) * lv.size - f.y
                    let d = (cx * cx + cy * cy).squareRoot()
                    guard d < lv.radius + lv.size * 0.72 else { continue }
                    wanted.insert(key)
                    if chunks[key] == nil { missing.append((d + Double(li) * 120, key)) }
                }
            }
        }
        for key in chunks.keys where !wanted.contains(key) { chunks[key] = nil }
        missing.sort { $0.0 < $1.0 }
        let n = fresh > 0 ? max(Self.budget, 12) : Self.budget
        fresh = max(fresh - 1, 0)
        for (_, key) in missing.prefix(n) { chunks[key] = make(key) }
    }

    private func make(_ key: Key) -> Chunk? {
        guard let sampler else { return nil }
        let lv = Self.levels[key.level]
        let n = Int((lv.size / lv.step).rounded()) + 1
        let m = n + 2  // one sample of border for the normals
        let origin = SIMD2(Double(key.i) * lv.size, Double(key.j) * lv.size)
        buffer = [Float](repeating: 0, count: 6 * m * m)
        let got = buffer.withUnsafeMutableBufferPointer {
            asmr_terrain_sample(sampler, origin.x - lv.step, origin.y - lv.step, Float(lv.step), UInt32(m), UInt32(m), $0.baseAddress)
        }
        guard got == m * m else { return nil }
        func at(_ i: Int, _ j: Int, _ c: Int) -> Float { buffer[6 * ((j + 1) * m + (i + 1)) + c] }
        let step = Float(lv.step)
        var ground = [Float](repeating: 0, count: n * n)
        var water = [Float](repeating: -1e9, count: n * n)
        var verts: [MeshVertex] = []
        verts.reserveCapacity(n * n)
        for j in 0..<n {
            for i in 0..<n {
                let g = at(i, j, 0)
                ground[j * n + i] = g
                water[j * n + i] = at(i, j, 1)
                let dx = at(i + 1, j, 0) - at(i - 1, j, 0)
                let dy = at(i, j + 1, 0) - at(i, j - 1, 0)
                let nrm = simd_normalize(SIMD3(-dx, -dy, 2 * step))
                var v = MeshVertex(SIMD3(Float(i) * step, Float(j) * step, g), nrm, Part.ground)
                // Height above the nearest water: beaches turn to sand in the shader.
                v.pad = min(at(i, j, 4), 999)
                verts.append(v)
            }
        }
        var idx: [UInt32] = []
        idx.reserveCapacity((n - 1) * (n - 1) * 6)
        for j in 0..<(n - 1) {
            for i in 0..<(n - 1) {
                let a = UInt32(j * n + i), b = a + 1, c = a + UInt32(n), d = c + 1
                // Split along the shorter diagonal (ridges and dune crests stay sharp).
                let ga = ground[Int(a)], gb = ground[Int(b)], gc = ground[Int(c)], gd = ground[Int(d)]
                if abs(ga - gd) < abs(gb - gc) { idx += [a, b, d, a, d, c] } else { idx += [a, b, c, b, d, c] }
            }
        }
        let groundMesh = Mesh.make(verts, idx)

        // Water: a flat quad per cell that has water at any corner, at that water level; each
        // vertex carries its depth (shore foam), the flow and the kind of water.
        var wv: [MeshVertex] = []
        var wi: [UInt32] = []
        for j in 0..<(n - 1) {
            for i in 0..<(n - 1) {
                let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)]
                var level: Float = -1e9
                var kind: Float = 0
                var flow = SIMD2<Float>(0, 0)
                for (ci, cj) in corners {
                    let w = at(ci, cj, 1)
                    if w > at(ci, cj, 0) && w > level {
                        level = w
                        kind = at(ci, cj, 5)
                        flow = SIMD2(at(ci, cj, 2), at(ci, cj, 3))
                    }
                }
                guard level > -1e8 else { continue }
                let base = UInt32(wv.count)
                for (ci, cj) in corners {
                    // Each wet vertex at its own level (shared with the next cell: no steps).
                    let w = at(ci, cj, 1)
                    let z = w > at(ci, cj, 0) ? w : level
                    var v = MeshVertex(SIMD3(Float(ci) * step, Float(cj) * step, z), SIMD3(flow.x, flow.y, kind), Part.water)
                    v.pad = z - at(ci, cj, 0)
                    wv.append(v)
                }
                wi += [base, base + 1, base + 2, base, base + 2, base + 3]
            }
        }
        let waterMesh = wv.isEmpty ? nil : Mesh.make(wv, wi)
        return Chunk(key: key, origin: origin, n: n, step: step, ground: ground, water: water, groundMesh: groundMesh, waterMesh: waterMesh)
    }

    /// A far chunk whose whole area is drawn by near chunks.
    func covered(_ c: Chunk) -> Bool {
        guard c.key.level == 1 else { return false }
        let k = Int(Self.levels[1].size / Self.levels[0].size)
        for a in 0..<k {
            for b in 0..<k where chunks[Key(level: 0, i: c.key.i * k + a, j: c.key.j * k + b)] == nil {
                return false
            }
        }
        return true
    }

    /// Model matrix of a chunk in the travel frame (ears at the origin).
    func model(_ c: Chunk) -> simd_float4x4 {
        let f = frame
        let off = SIMD2<Float>(Float(c.origin.x - f.x), Float(c.origin.y - f.y))
        let (s, co) = (sin(-f.heading), cos(-f.heading))
        let sink = Self.levels[c.key.level].sink
        // rotate(−heading) · translate(origin − listener, −ears)
        var m = simd_float4x4(1)
        m.columns.0 = SIMD4(co, s, 0, 0)
        m.columns.1 = SIMD4(-s, co, 0, 0)
        m.columns.2 = SIMD4(0, 0, 1, 0)
        let t = SIMD2(co * off.x - s * off.y, s * off.x + co * off.y)
        m.columns.3 = SIMD4(t.x, t.y, -f.ear_z - sink, 1)
        return m
    }

    /// Ground (or water surface) height at a travel-frame point, relative to the ears; nil
    /// where no near chunk is ready.
    func height(_ x: Float, _ y: Float, water: Bool = false) -> Float? {
        let f = frame
        let (s, c) = (sin(f.heading), cos(f.heading))
        let wx = f.x + Double(c * x - s * y), wy = f.y + Double(s * x + c * y)
        let lv = Self.levels[0]
        let key = Key(level: 0, i: Int((wx / lv.size).rounded(.down)), j: Int((wy / lv.size).rounded(.down)))
        guard let ch = chunks[key] else { return nil }
        let fx = Float((wx - ch.origin.x) / lv.step), fy = Float((wy - ch.origin.y) / lv.step)
        let i0 = min(max(Int(fx), 0), ch.n - 2), j0 = min(max(Int(fy), 0), ch.n - 2)
        let u = min(max(fx - Float(i0), 0), 1), v = min(max(fy - Float(j0), 0), 1)
        func g(_ i: Int, _ j: Int) -> Float {
            let k = j * ch.n + i
            return water ? max(ch.ground[k], ch.water[k]) : ch.ground[k]
        }
        let a = g(i0, j0) + (g(i0 + 1, j0) - g(i0, j0)) * u
        let b = g(i0, j0 + 1) + (g(i0 + 1, j0 + 1) - g(i0, j0 + 1)) * u
        return a + (b - a) * v - f.ear_z
    }
}
