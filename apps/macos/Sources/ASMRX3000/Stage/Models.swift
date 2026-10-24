import Foundation
import Metal
import simd

/// Vertex layout shared with Blender's exporter (tools/blender/build_models.py) and the shaders.
/// Packed: 8 floats, 32 bytes (SIMD3 would pad to 16 bytes each).
struct MeshVertex {
    var px, py, pz, nx, ny, nz, part, pad: Float

    init(_ p: SIMD3<Float>, _ n: SIMD3<Float>, _ part: Float) {
        (px, py, pz) = (p.x, p.y, p.z)
        (nx, ny, nz) = (n.x, n.y, n.z)
        self.part = part
        pad = 0
    }

    var position: SIMD3<Float> { SIMD3(px, py, pz) }
}

/// Parts, mirrored in the shaders.
enum Part {
    static let wood: Float = 0, foliage: Float = 1, feathers: Float = 2, wingL: Float = 3, wingR: Float = 4
    static let stone: Float = 5, beak: Float = 6, ground: Float = 7, water: Float = 8, section: Float = 9
    /// A clip scene's place (ClipSets.swift): pale linework that takes no contour or selection.
    static let set: Float = 10
}

final class Mesh {
    let vertices: MTLBuffer
    let indices: MTLBuffer
    let indexCount: Int

    init?(device: MTLDevice, vertices: UnsafeRawBufferPointer, indices: UnsafeRawBufferPointer) {
        guard let v = device.makeBuffer(bytes: vertices.baseAddress!, length: vertices.count, options: .storageModeShared),
              let i = device.makeBuffer(bytes: indices.baseAddress!, length: indices.count, options: .storageModeShared)
        else { return nil }
        self.vertices = v
        self.indices = i
        indexCount = indices.count / 4
    }

    static func make(_ vertices: [MeshVertex], _ indices: [UInt32]) -> Mesh? {
        vertices.withUnsafeBytes { v in indices.withUnsafeBytes { i in Mesh(device: GPU.shared.device, vertices: v, indices: i) } }
    }
}

/// The modelled objects (trees, birds, the herm, the ruin), loaded from `models.bin`.
final class ModelLibrary {
    static let shared = ModelLibrary()

    private(set) var meshes: [String: Mesh] = [:]

    private init() {
        guard let url = Self.locate(), let data = try? Data(contentsOf: url) else {
            NSLog("ASMR X3000: models.bin not found; the scene will be empty")
            return
        }
        data.withUnsafeBytes { raw in
            guard raw.count > 12, raw.prefix(8).elementsEqual("ASMRMDL1".utf8) else { return }
            var off = 8
            func u32() -> Int {
                let v = raw.loadUnaligned(fromByteOffset: off, as: UInt32.self)
                off += 4
                return Int(v)
            }
            let count = u32()
            for _ in 0..<count {
                let nameBytes = raw[off..<off + 32].prefix { $0 != 0 }
                let name = String(decoding: nameBytes, as: UTF8.self)
                off += 32
                let nv = u32(), ni = u32()
                let v = UnsafeRawBufferPointer(rebasing: raw[off..<off + nv * 32])
                off += nv * 32
                let i = UnsafeRawBufferPointer(rebasing: raw[off..<off + ni * 4])
                off += ni * 4
                meshes[name] = Mesh(device: GPU.shared.device, vertices: v, indices: i)
            }
        }
    }

    /// Inside the app bundle, else next to the sources (development runs from .build).
    private static func locate() -> URL? {
        if let url = Bundle.main.url(forResource: "models", withExtension: "bin") { return url }
        let src = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../Resources/models.bin").standardizedFileURL
        return FileManager.default.fileExists(atPath: src.path) ? src : nil
    }
}

/// The ground disc with its river channel and the cut face around it, rebuilt on the CPU
/// whenever the river moves.
enum TerrainMesh {
    static let rings = 110
    static let segments = 300
    static let bottom: Float = -3.4

    /// Ground height including the river channel, and the water level there.
    static func height(_ x: Float, _ y: Float, river: [SIMD4<Float>]) -> (ground: Float, water: Float, inChannel: Bool) {
        let rv = Stage.riverAt(y, river)
        let dist = abs(x - rv.x)
        let wl = Terrain.waterLevel(rv.y)
        let t = min(max((dist - 0.5) / 2.0, 0), 1)
        let bank = t * t * (3 - 2 * t)
        let bed = wl - 0.42 + 0.12 * dist
        // Outside the channel the ground stays dry, never below the water line.
        let ground = bed + (Terrain.farHeight(x, y) - bed) * bank
        return dist < 2.2 ? (ground, wl, true) : (max(ground, wl + 0.05), wl, false)
    }

    static func build(river: [SIMD4<Float>]) -> Mesh? {
        GroundMesh.build(segments: segments) { x, y in
            let h = height(x, y, river: river)
            return h.inChannel && h.ground < h.water - 0.02 ? (h.water, Part.water) : (h.ground, Part.ground)
        }
    }
}

/// Ground that runs on to the horizon: a polar grid, dense over the play area and opening up
/// geometrically beyond it, with no rim or underside (the haze in the engraving swallows the
/// far edge). Shared by the scene's terrain and anything that streams ground (flight mode):
/// give it a `surface(x, y) -> (z, part)`.
enum GroundMesh {
    /// Rings over the play area (as dense as the old disc) and beyond it.
    static let innerRings = 110
    static let outerRings = 44
    static let farRadius: Float = 220

    static func radius(ring r: Int, inner: Int = innerRings, outer: Int = outerRings) -> Float {
        if r <= inner { return Terrain.radius * Float(r) / Float(inner) }
        // Geometric: each ring a fixed ratio further, so screen-space spacing stays even.
        let step = Terrain.radius / Float(inner)
        let k = Float(r - inner) / Float(outer)
        let ratio = farRadius / Terrain.radius
        return max(Terrain.radius * pow(ratio, k), Terrain.radius + step * Float(r - inner))
    }

    /// `inner`/`outer`: ring counts over and beyond the play area (coarser for ground rebuilt
    /// every frame).
    static func build(segments: Int, inner: Int = innerRings, outer: Int = outerRings,
                      surface: (Float, Float) -> (Float, Float)) -> Mesh? {
        let rings = inner + outer
        var verts: [MeshVertex] = []
        var idx: [UInt32] = []
        verts.reserveCapacity(rings * segments + 1)
        func normal(_ x: Float, _ y: Float, _ e: Float) -> SIMD3<Float> {
            let dx = surface(x + e, y).0 - surface(x - e, y).0
            let dy = surface(x, y + e).0 - surface(x, y - e).0
            return simd_normalize(SIMD3(-dx, -dy, 2 * e))
        }
        let (cz, cp) = surface(0, 0)
        verts.append(MeshVertex(SIMD3(0, 0, cz), normal(0, 0, 0.08), cp))
        for r in 1...rings {
            let radius = Self.radius(ring: r, inner: inner, outer: outer)
            let e = max(0.08, (radius - Self.radius(ring: r - 1, inner: inner, outer: outer)) * 0.5)
            for s in 0..<segments {
                let a = 2 * Float.pi * Float(s) / Float(segments)
                let x = radius * cos(a), y = radius * sin(a)
                let (z, part) = surface(x, y)
                verts.append(MeshVertex(SIMD3(x, y, z), normal(x, y, e), part))
            }
        }
        func ring(_ r: Int, _ s: Int) -> UInt32 { UInt32(1 + (r - 1) * segments + (s % segments)) }
        for s in 0..<segments { idx += [0, ring(1, s), ring(1, s + 1)] }
        for r in 1..<rings {
            for s in 0..<segments {
                let a = ring(r, s), b = ring(r, s + 1), c = ring(r + 1, s), d = ring(r + 1, s + 1)
                idx += [a, c, b, b, c, d]
            }
        }
        return Mesh.make(verts, idx)
    }
}
