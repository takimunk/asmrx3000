import CoreGraphics
import Foundation
import ImageIO
import Metal
import simd

// Her head and shoulders as a real mesh (Resources/Onboarding/head.bin, built offline by
// tools/blender/build_head.py from a fal.ai image-to-3D generation of the front portrait), drawn
// by Metal into a small offscreen buffer and read back: per pixel the drawing's luminance (its
// texture), how far into the shadow side the surface turns (a hard side light from the screen's
// left) and the depth. Dither.swift turns that into the same paper / shade / void cells as before.
// Above the neck the mesh turns with the pose about a pivot in her neck; the shoulders stay put.
// Self-contained (its own device and queue), so frames can also be rendered outside the app.

/// What the offscreen pass gives back, `size × size`, row-major, top row first.
struct HeadBuffer {
    let size: Int
    /// The drawing's luminance (0 black … 1 white).
    var lum: [Float]
    /// 0 lit … 1 deep in the shadow side.
    var shade: [Float]
    /// Toward the viewer, model units (the image square spans −0.5 … 0.5).
    var depth: [Float]
    /// 1 where the mesh covers the pixel.
    var cover: [Float]
}

final class HeadModel {
    private struct Uniforms {
        var rot: simd_float4x4
        var pivot: SIMD4<Float>   // xyz, w unused
        var neck: SIMD4<Float>    // weight 0 below x, 1 above y
        var light: SIMD4<Float>   // direction xyz (towards the light), w the terminator's N·L
    }

    /// Points on the mesh (model space at rest): the ears (the earbuds) and the eye nearest the
    /// viewer in profile, the turn's pivot and the neck's blend band.
    struct Markers {
        var earLeft = SIMD3<Float>(0.17, 0.03, 0.0)   // her left ear: the screen's right at yaw 0
        var earRight = SIMD3<Float>(-0.2, 0.03, 0.0)
        var eyeRight = SIMD3<Float>(-0.08, 0.13, 0.15)
        var pivot = SIMD3<Float>(-0.01, -0.12, -0.03)
        var neck = SIMD2<Float>(-0.26, -0.12)
    }

    static let shared: HeadModel? = HeadModel()

    let markers: Markers
    private let device: MTLDevice
    private let queue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private let depthState: MTLDepthStencilState
    private let vertices: MTLBuffer
    private let indices: MTLBuffer
    private let indexCount: Int
    private let texture: MTLTexture
    private let sampler: MTLSamplerState
    private var targets: (size: Int, color: MTLTexture, depth: MTLTexture, readback: MTLBuffer)?

    static var url: URL { Portraits.directory.appendingPathComponent("head.bin") }

    private init?() {
        guard let data = try? Data(contentsOf: Self.url),
              let device = MTLCreateSystemDefaultDevice(), let queue = device.makeCommandQueue() else { return nil }
        // head.bin (see build_head.py): b"ASMRHEAD", u32 version, u32 vertex count, u32 index
        // count, u32 reserved, 16 floats of markers, the vertices (8 halves: position, normal,
        // uv; handed to the GPU as they are), u16 indices. head.png: the texture's luminance.
        var m = Markers()
        var verts = Data(), idx = Data()
        let ok: Bool = data.withUnsafeBytes { raw in
            guard raw.count > 88, raw.prefix(8).elementsEqual("ASMRHEAD".utf8) else { return false }
            var off = 8
            func u32() -> Int { defer { off += 4 }; return Int(raw.loadUnaligned(fromByteOffset: off, as: UInt32.self)) }
            func f32() -> Float { defer { off += 4 }; return raw.loadUnaligned(fromByteOffset: off, as: Float.self) }
            guard u32() == 2 else { return false }
            let nv = u32(), ni = u32()
            _ = u32()
            func v3() -> SIMD3<Float> { SIMD3(f32(), f32(), f32()) }
            m.earLeft = v3(); m.earRight = v3(); m.eyeRight = v3(); m.pivot = v3()
            m.neck = SIMD2(f32(), f32())
            _ = f32(); _ = f32()
            guard raw.count >= off + nv * 16 + ni * 2 else { return false }
            verts = Data(raw[off..<(off + nv * 16)])
            off += nv * 16
            idx = Data(raw[off..<(off + ni * 2)])
            return true
        }
        guard ok, !idx.isEmpty, let (tex, side) = Self.luminance(Portraits.directory.appendingPathComponent("head.png")) else { return nil }
        markers = m
        self.device = device
        self.queue = queue

        guard let library = try? device.makeLibrary(source: Self.shader, options: nil) else { return nil }
        let d = MTLRenderPipelineDescriptor()
        d.vertexFunction = library.makeFunction(name: "head_vs")
        d.fragmentFunction = library.makeFunction(name: "head_fs")
        d.colorAttachments[0].pixelFormat = .rgba8Unorm
        d.depthAttachmentPixelFormat = .depth32Float
        guard let p = try? device.makeRenderPipelineState(descriptor: d) else { return nil }
        pipeline = p
        let ds = MTLDepthStencilDescriptor()
        ds.depthCompareFunction = .less
        ds.isDepthWriteEnabled = true
        guard let dss = device.makeDepthStencilState(descriptor: ds) else { return nil }
        depthState = dss

        guard let vb = verts.withUnsafeBytes({ device.makeBuffer(bytes: $0.baseAddress!, length: $0.count) }),
              let ib = idx.withUnsafeBytes({ device.makeBuffer(bytes: $0.baseAddress!, length: $0.count) }) else { return nil }
        vertices = vb
        indices = ib
        indexCount = idx.count / 2

        let td = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .r8Unorm, width: side, height: side, mipmapped: true)
        td.usage = .shaderRead
        guard let t = device.makeTexture(descriptor: td) else { return nil }
        tex.withUnsafeBytes { t.replace(region: MTLRegionMake2D(0, 0, side, side), mipmapLevel: 0, withBytes: $0.baseAddress!, bytesPerRow: side) }
        if let cb = queue.makeCommandBuffer(), let blit = cb.makeBlitCommandEncoder() {
            blit.generateMipmaps(for: t)
            blit.endEncoding()
            cb.commit()
            cb.waitUntilCompleted()
        }
        texture = t
        let sd = MTLSamplerDescriptor()
        sd.minFilter = .linear; sd.magFilter = .linear; sd.mipFilter = .linear
        sd.sAddressMode = .clampToEdge; sd.tAddressMode = .clampToEdge
        guard let s = device.makeSamplerState(descriptor: sd) else { return nil }
        sampler = s
    }

    /// A grey image as bytes, square.
    private static func luminance(_ url: URL) -> ([UInt8], Int)? {
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let img = CGImageSourceCreateImageAtIndex(src, 0, nil), img.width == img.height else { return nil }
        let n = img.width
        var px = [UInt8](repeating: 0, count: n * n)
        guard let ctx = CGContext(data: &px, width: n, height: n, bitsPerComponent: 8, bytesPerRow: n,
                                  space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return nil }
        ctx.draw(img, in: CGRect(x: 0, y: 0, width: n, height: n))
        return (px, n)
    }

    /// The head's turn: yaw + turns her nose to the screen's left (the mirror image), pitch +
    /// lifts it, roll + lowers the screen's right side.
    static func rotation(_ pose: HeadPose) -> simd_float3x3 {
        let y = Float(-pose.yaw), p = Float(-pose.pitch), r = Float(-pose.roll)
        let ry = simd_float3x3(rows: [SIMD3(cos(y), 0, sin(y)), SIMD3(0, 1, 0), SIMD3(-sin(y), 0, cos(y))])
        let rx = simd_float3x3(rows: [SIMD3(1, 0, 0), SIMD3(0, cos(p), -sin(p)), SIMD3(0, sin(p), cos(p))])
        let rz = simd_float3x3(rows: [SIMD3(cos(r), -sin(r), 0), SIMD3(sin(r), cos(r), 0), SIMD3(0, 0, 1)])
        return ry * rx * rz
    }

    /// Where a point of the mesh at rest goes for this pose (the same blend as the vertex shader).
    func place(_ p: SIMD3<Float>, _ pose: HeadPose) -> SIMD3<Float> {
        let w = smoothstep(markers.neck.x, markers.neck.y, p.y)
        let r = Self.rotation(pose) * (p - markers.pivot) + markers.pivot
        return p + (r - p) * w
    }

    /// The light, towards it in view space (x right, y up, z to the viewer), and the N·L where
    /// the shadow side begins. `fromRight` mirrors it (the profile faces the screen on the right).
    static func light(fromRight: Bool = false) -> SIMD4<Float> {
        let l = simd_normalize(SIMD3<Float>(fromRight ? 0.5 : -0.5, 0.3, 0.81))
        return SIMD4(l, 0.22)
    }

    /// Draws the mesh for `pose` into a `size × size` buffer showing the image square.
    func render(_ pose: HeadPose, size: Int, light: SIMD4<Float> = HeadModel.light()) -> HeadBuffer? {
        if targets?.size != size {
            let cd = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm, width: size, height: size, mipmapped: false)
            cd.usage = .renderTarget
            cd.storageMode = .private
            let dd = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .depth32Float, width: size, height: size, mipmapped: false)
            dd.usage = .renderTarget
            dd.storageMode = .private
            guard let c = device.makeTexture(descriptor: cd), let d = device.makeTexture(descriptor: dd),
                  let b = device.makeBuffer(length: size * size * 4, options: .storageModeShared) else { return nil }
            targets = (size, c, d, b)
        }
        guard let t = targets, let cb = queue.makeCommandBuffer() else { return nil }
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = t.color
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        pass.colorAttachments[0].storeAction = .store
        pass.depthAttachment.texture = t.depth
        pass.depthAttachment.loadAction = .clear
        pass.depthAttachment.clearDepth = 1
        pass.depthAttachment.storeAction = .dontCare
        guard let enc = cb.makeRenderCommandEncoder(descriptor: pass) else { return nil }
        let r = Self.rotation(pose)
        var u = Uniforms(rot: simd_float4x4(SIMD4(r.columns.0, 0), SIMD4(r.columns.1, 0), SIMD4(r.columns.2, 0), SIMD4(0, 0, 0, 1)),
                         pivot: SIMD4(markers.pivot, 0), neck: SIMD4(markers.neck.x, markers.neck.y, 0, 0), light: light)
        enc.setRenderPipelineState(pipeline)
        enc.setDepthStencilState(depthState)
        enc.setCullMode(.none)
        enc.setVertexBuffer(vertices, offset: 0, index: 0)
        enc.setVertexBytes(&u, length: MemoryLayout<Uniforms>.stride, index: 1)
        enc.setFragmentBytes(&u, length: MemoryLayout<Uniforms>.stride, index: 1)
        enc.setFragmentTexture(texture, index: 0)
        enc.setFragmentSamplerState(sampler, index: 0)
        enc.drawIndexedPrimitives(type: .triangle, indexCount: indexCount, indexType: .uint16, indexBuffer: indices, indexBufferOffset: 0)
        enc.endEncoding()
        // Copied into a shared buffer to read it back (works on every GPU).
        if let blit = cb.makeBlitCommandEncoder() {
            blit.copy(from: t.color, sourceSlice: 0, sourceLevel: 0, sourceOrigin: MTLOrigin(x: 0, y: 0, z: 0),
                      sourceSize: MTLSize(width: size, height: size, depth: 1), to: t.readback,
                      destinationOffset: 0, destinationBytesPerRow: size * 4, destinationBytesPerImage: size * size * 4)
            blit.endEncoding()
        }
        cb.commit()
        cb.waitUntilCompleted()

        let n = size * size
        let px = t.readback.contents().bindMemory(to: UInt8.self, capacity: n * 4)
        var b = HeadBuffer(size: size, lum: .init(repeating: 0, count: n), shade: .init(repeating: 0, count: n),
                           depth: .init(repeating: -1, count: n), cover: .init(repeating: 0, count: n))
        for i in 0..<n where px[i * 4 + 3] > 0 {
            b.lum[i] = Float(px[i * 4]) / 255
            b.shade[i] = Float(px[i * 4 + 1]) / 255
            b.depth[i] = Float(px[i * 4 + 2]) / 255 - 0.5
            b.cover[i] = 1
        }
        return b
    }

    private func smoothstep(_ a: Float, _ b: Float, _ x: Float) -> Float {
        let t = max(0, min(1, (x - a) / (b - a)))
        return t * t * (3 - 2 * t)
    }

    // Orthographic: the image square (−0.5 … 0.5 in x and y) fills the target; z towards the
    // viewer, nearer is smaller depth.
    private static let shader = """
    #include <metal_stdlib>
    using namespace metal;
    struct V { packed_half3 p; packed_half3 n; packed_half2 uv; };
    struct U { float4x4 rot; float4 pivot; float4 neck; float4 light; };
    struct O { float4 pos [[position]]; float3 n; float2 uv; float z; };

    vertex O head_vs(uint vid [[vertex_id]], const device V* vs [[buffer(0)]], constant U& u [[buffer(1)]]) {
        float3 p = float3(vs[vid].p), n = float3(vs[vid].n);
        float w = smoothstep(u.neck.x, u.neck.y, p.y);
        float3 pr = (u.rot * float4(p - u.pivot.xyz, 0)).xyz + u.pivot.xyz;
        float3 nr = (u.rot * float4(n, 0)).xyz;
        p = mix(p, pr, w);
        n = normalize(mix(n, nr, w));
        O o;
        o.pos = float4(p.x * 2, p.y * 2, 0.5 - p.z * 0.5, 1);
        o.n = n;
        o.uv = float2(vs[vid].uv);
        o.z = p.z;
        return o;
    }

    fragment float4 head_fs(O i [[stage_in]], texture2d<float> t [[texture(0)]], sampler s [[sampler(0)]],
                            constant U& u [[buffer(1)]]) {
        float lum = t.sample(s, i.uv, bias(-0.5)).r;
        float3 n = normalize(i.n);
        float d = dot(n, u.light.xyz);
        float shade = 1 - smoothstep(u.light.w - 0.1, u.light.w + 0.1, d);
        return float4(lum, shade, clamp(i.z + 0.5, 0.0, 1.0), 1);
    }
    """
}
