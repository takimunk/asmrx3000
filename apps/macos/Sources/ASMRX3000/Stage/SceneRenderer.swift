import CAsmr
import Metal
import simd

private struct FrameUniforms {
    var viewProj: simd_float4x4
    var sunViewProj: simd_float4x4
    var cam: SIMD4<Float>
    var view: SIMD4<Float>
    var head: SIMD4<Float>
    var counts: SIMD4<Float>
    var bracket: SIMD4<Float>
    var hoverBracket: SIMD4<Float>
    var plumb: SIMD4<Float>
    var sun: SIMD4<Float>
    var env: SIMD4<Float>
    var gaze: SIMD4<Float>      // dot screen x, y px, px per metre, live (1) or resting (0)
    var gazeHead: SIMD4<Float>  // head screen x, y px, dot distance from the eye, visible
    var haze: SIMD4<Float>      // horizon y px
}

private struct Instance {
    var model: simd_float4x4
    var info: SIMD4<Float>   // id, flags, lean, model height
    var anim: SIMD4<Float>   // wing angle, phase, local wind, kind (1 = tree)
}

/// Draws the scene: shadow map → G-buffer of the Blender meshes and terrain → engraving.
final class SceneRenderer {
    private let stage: Stage
    private let shadowPipeline: MTLRenderPipelineState
    private let gbufPipeline: MTLRenderPipelineState
    private let engravePipeline: MTLRenderPipelineState
    private let depthState: MTLDepthStencilState
    private let shadowMap: MTLTexture
    private var gA: MTLTexture?, gB: MTLTexture?, gDepth: MTLTexture?

    private var terrain: Mesh?
    private var terrainRiver: [SIMD4<Float>] = []
    private let pole: Mesh?
    /// Flight mode: the scrolling ground (rebuilt every frame) and a boulder.
    private var flightGround: Mesh?
    private let boulder = FlightGround.makeBoulder()
    private var data = [SIMD4<Float>](repeating: .zero, count: 48)
    private var lastBird: [UInt32: SIMD3<Float>] = [:]
    private var heading: [UInt32: Float] = [:]

    static let sun = simd_normalize(SIMD3<Float>(-0.42, 0.5, 0.76))
    static let birdScale: Float = 4.0
    static let hermScale: Float = 1.6
    /// Trees are drawn narrower than life so the ground, river and herm stay in view.
    static let slender = simd_float4x4(diagonal: SIMD4(0.62, 0.62, 1, 1))
    /// Where the fallen column lies, on the far bank.
    static let ruin = SIMD3<Float>(7.0, -8.6, 0)

    /// Trees scattered from just beyond the play area out to ~70 m, denser near: x, y, scale, model.
    private static let distantForest: [(Float, Float, Float, String)] = {
        var rng = SeededGenerator(0x5EED)
        let names = ["broadleaf", "spruce", "broadleaf_b", "spruce_b", "spruce"]
        return (0..<56).map { i in
            let a = Float(i) * 2.399963 + rng.next() * 0.4
            let r = 24 + pow(rng.next(), 1.4) * 60
            return (r * cos(a), r * sin(a), 0.8 + rng.next() * 0.45, names[i % names.count])
        }
    }()

    /// Decorative grove: x, y, scale, model. Never selectable.
    private static let grove: [(Float, Float, Float, String)] = [
        (-6.0, 8.6, 0.85, "spruce"), (-9.6, 3.0, 0.8, "broadleaf_b"), (9.8, 7.4, 0.9, "spruce"),
        (11.2, 0.4, 0.85, "broadleaf_b"), (9.4, -7.8, 0.8, "spruce_b"), (-1.2, 11.6, 0.8, "spruce"),
        (4.0, 11.2, 0.75, "broadleaf"),
    ]

    init(stage: Stage, format: MTLPixelFormat = .bgra8Unorm) {
        self.stage = stage
        let gpu = GPU.shared
        let device = gpu.device

        let sd = MTLRenderPipelineDescriptor()
        sd.vertexFunction = gpu.library.makeFunction(name: "shadowVert")
        sd.depthAttachmentPixelFormat = .depth32Float
        shadowPipeline = try! device.makeRenderPipelineState(descriptor: sd)

        let gd = MTLRenderPipelineDescriptor()
        gd.vertexFunction = gpu.library.makeFunction(name: "gbufVert")
        gd.fragmentFunction = gpu.library.makeFunction(name: "gbufFrag")
        gd.colorAttachments[0].pixelFormat = .rgba16Float
        gd.colorAttachments[1].pixelFormat = .rgba16Float
        gd.depthAttachmentPixelFormat = .depth32Float
        gbufPipeline = try! device.makeRenderPipelineState(descriptor: gd)

        engravePipeline = gpu.pipeline(fragment: "engrave", format: format)

        let ds = MTLDepthStencilDescriptor()
        ds.depthCompareFunction = .less
        ds.isDepthWriteEnabled = true
        depthState = device.makeDepthStencilState(descriptor: ds)!

        let td = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .depth32Float, width: 2048, height: 2048, mipmapped: false)
        td.usage = [.renderTarget, .shaderRead]
        td.storageMode = .private
        shadowMap = device.makeTexture(descriptor: td)!

        pole = Self.makePole()
    }

    func encode(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {
        ensureTargets(width: target.width, height: target.height)
        let river = stage.riverPoints()
        if stage.flight.active {
            flightGround = FlightGround.build(stage.flight)
        } else if terrain == nil || river.map({ SIMD3($0.x, $0.y, $0.z) }) != terrainRiver.map({ SIMD3($0.x, $0.y, $0.z) }) {
            terrain = TerrainMesh.build(river: river)
            terrainRiver = river
        }
        guard let gA, let gB, let gDepth else { return }

        let cam = stage.camera
        let vp = cam.viewProjection(aspect: size.x / size.y)
        let proj = Projector(camera: cam, size: size)
        let time = Float(stage.time.truncatingRemainder(dividingBy: 3600))
        var u = FrameUniforms(
            viewProj: vp, sunViewProj: Self.sunMatrix(),
            cam: SIMD4(cam.eye, time),
            view: SIMD4(Float(target.width), Float(target.height), scale, cam.fovY / Float(target.height)),
            head: SIMD4(stage.headPose.yaw, 0, 0, stage.audio.isPlaying ? 1 : 0),
            counts: .zero,
            bracket: SIMD4(0, 0, -1, -1), hoverBracket: SIMD4(0, 0, -1, -1),
            plumb: .zero, sun: SIMD4(Self.sun, 0),
            env: SIMD4(Self.daylight(stage.hour), stage.telemetry.wetness, stage.telemetry.lightning, 0),
            gaze: .zero, gazeHead: .zero, haze: .zero
        )
        // The horizon: where level ground meets the sky, straight ahead of the camera.
        let look = cam.target - cam.eye
        let level = simd_normalize(SIMD3(look.x, look.y, 0))
        u.haze.x = (proj.project(cam.eye + level * 5000).map { $0.point.y } ?? -1000) * scale
        let gazeAt = stage.gazePoint
        if let d = proj.project(gazeAt), let h = proj.project(SIMD3(0, 0, 0)) {
            let focal = size.y * 0.5 / tan(cam.fovY / 2)
            u.gaze = SIMD4(d.point.x * scale, d.point.y * scale, focal / d.w * scale, stage.headPose.live ? 1 : 0)
            u.gazeHead = SIMD4(h.point.x * scale, h.point.y * scale, simd_distance(cam.eye, gazeAt), 1)
        }
        let instances = buildInstances(time: time, river: river, proj: proj, scale: scale, u: &u)
        if let t = stage.thing(stage.selection) {
            if let b = proj.bounds(min: t.boxMin, max: t.boxMax) { u.bracket = snapped(b, scale: scale) }
            if t.kind == .bird, let a = proj.project(t.anchor),
               let g = proj.project(SIMD3(t.anchor.x, t.anchor.y, Terrain.height(t.anchor.x, t.anchor.y))) {
                let pa = a.point * scale, pg = g.point * scale
                u.plumb = SIMD4(pa.x, pa.y, pg.x, pg.y)
                u.sun.w = 1
            }
        }
        if stage.hover != stage.selection, let t = stage.thing(stage.hover), let b = proj.bounds(min: t.boxMin, max: t.boxMax) {
            u.hoverBracket = snapped(b, scale: scale)
        }

        // 1. Shadows.
        let sp = MTLRenderPassDescriptor()
        sp.depthAttachment.texture = shadowMap
        sp.depthAttachment.loadAction = .clear
        sp.depthAttachment.clearDepth = 1
        sp.depthAttachment.storeAction = .store
        if let enc = cb.makeRenderCommandEncoder(descriptor: sp) {
            enc.setRenderPipelineState(shadowPipeline)
            enc.setDepthStencilState(depthState)
            enc.setDepthBias(0.002, slopeScale: 2, clamp: 0.01)
            draw(instances, enc: enc, u: &u)
            enc.endEncoding()
        }

        // 2. G-buffer.
        let gp = MTLRenderPassDescriptor()
        for (i, tex) in [gA, gB].enumerated() {
            gp.colorAttachments[i].texture = tex
            gp.colorAttachments[i].loadAction = .clear
            gp.colorAttachments[i].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
            gp.colorAttachments[i].storeAction = .store
        }
        gp.colorAttachments[1].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: -1)
        gp.depthAttachment.texture = gDepth
        gp.depthAttachment.loadAction = .clear
        gp.depthAttachment.clearDepth = 1
        gp.depthAttachment.storeAction = .dontCare
        if let enc = cb.makeRenderCommandEncoder(descriptor: gp) {
            enc.setRenderPipelineState(gbufPipeline)
            enc.setDepthStencilState(depthState)
            enc.setFragmentTexture(shadowMap, index: 0)
            draw(instances, enc: enc, u: &u)
            enc.endEncoding()
        }

        // 3. Engraving.
        GPU.fullscreenPass(cb, target: target, pipeline: engravePipeline) { enc in
            enc.setFragmentBytes(&u, length: MemoryLayout<FrameUniforms>.stride, index: 0)
            data.withUnsafeBytes { enc.setFragmentBytes($0.baseAddress!, length: $0.count, index: 1) }
            enc.setFragmentTexture(gA, index: 0)
            enc.setFragmentTexture(gB, index: 1)
        }
    }

    // MARK: Instances

    private func draw(_ list: [(Mesh, Instance)], enc: MTLRenderCommandEncoder, u: inout FrameUniforms) {
        enc.setVertexBytes(&u, length: MemoryLayout<FrameUniforms>.stride, index: 2)
        enc.setFragmentBytes(&u, length: MemoryLayout<FrameUniforms>.stride, index: 2)
        for (mesh, inst) in list {
            var i = inst
            enc.setVertexBuffer(mesh.vertices, offset: 0, index: 0)
            enc.setVertexBytes(&i, length: MemoryLayout<Instance>.stride, index: 1)
            enc.setFragmentBytes(&i, length: MemoryLayout<Instance>.stride, index: 1)
            enc.drawIndexedPrimitives(type: .triangle, indexCount: mesh.indexCount, indexType: .uint32,
                                      indexBuffer: mesh.indices, indexBufferOffset: 0)
        }
    }

    private func flags(_ key: Thing.Key) -> Float {
        if stage.selection == key { return 1 }
        if stage.hover == key { return 2 }
        return 0
    }

    private func buildInstances(time: Float, river: [SIMD4<Float>], proj: Projector, scale: Float,
                                u: inout FrameUniforms) -> [(Mesh, Instance)] {
        let lib = ModelLibrary.shared.meshes
        var list: [(Mesh, Instance)] = []
        for i in data.indices { data[i] = .zero }
        for (i, p) in river.prefix(8).enumerated() { data[i] = p }
        for i in 0..<64 { data[16 + i / 4][i % 4] = stage.wind[i] }

        let flight = stage.flight
        let flying = flight.active
        // In flight the ground sits `drop` below the herm, which flies at the altitude.
        func ground(_ x: Float, _ y: Float) -> Float {
            flying ? FlightGround.height(x, y, odometer: flight.odometer) - flight.drop : Terrain.height(x, y)
        }
        if flying, let flightGround {
            let m = Self.transform(SIMD3(0, 0, -flight.drop), yaw: 0, scale: 1)
            list.append((flightGround, Instance(model: m, info: SIMD4(-1, 0, 0, 0), anim: .zero)))
        } else if let terrain {
            list.append((terrain, Instance(model: matrix_identity_float4x4, info: SIMD4(-1, flags(.river), 0, 0), anim: .zero)))
        }

        var treeIndex = 0, birds = 0
        let focal = proj.size.y * 0.5 / tan(stage.camera.fovY / 2)
        for o in stage.objects {
            let p = SIMD3(o.x, o.y, o.z)
            if o.flags & UInt32(ASMR_OBJECT_FLAG_ABSENT) != 0 { continue }
            switch o.kind {
            case UInt32(ASMR_OBJECT_TREE):
                let names = ["broadleaf", "spruce", "broadleaf_b", "spruce_b"]
                // Streamed trees keep their model by id; the scene's go in order.
                let name = names[(flying ? Int(o.id % 4) : treeIndex) % names.count]
                treeIndex += 1
                let r = simd_length(SIMD2(o.x, o.y))
                if flying && r > Terrain.radius - 0.3 { continue }
                guard let mesh = lib[name] else { continue }
                let g = ground(o.x, o.y)
                let crown: Float = name.hasPrefix("spruce") ? 3.6 : 4.2
                // Streamed trees rise out of the disc's rim and sink back into it behind.
                let rim: Float = flying ? min(max((Terrain.radius - 0.3 - r) / 1.6, 0), 1) : 1
                let s = (Stage.treeHeight * 0.75 + sin(Float(o.id) * 2.1) * 1.2) / crown * (0.25 + 0.75 * rim)
                let wind = localWind(o.y)
                let m = Self.transform(SIMD3(o.x, o.y, g), yaw: Float(o.id) * 1.3, scale: s) * Self.slender
                list.append((mesh, Instance(model: m, info: SIMD4(Float(o.id), flags(.object(o.id)), wind * 0.55, 6.6),
                                            anim: SIMD4(0, Float(o.id), wind, 1))))
            case UInt32(ASMR_OBJECT_ROCK) where flying:
                let g = ground(o.x, o.y)
                guard let boulder, simd_length(SIMD2(o.x, o.y)) < Terrain.radius - 0.8 else { continue }
                let radius = min(max((o.z + flight.altitude) / 1.4, 0.4), 2.0)
                let m = Self.transform(SIMD3(o.x, o.y, g - 0.15), yaw: Float(o.id % 7), scale: radius)
                list.append((boulder, Instance(model: m, info: SIMD4(-1, 0, 0, 0), anim: .zero)))
            case UInt32(ASMR_OBJECT_BIRD):
                if flying && simd_length(SIMD2(o.x, o.y)) > Terrain.radius + 1 { continue }
                // The bird's voice is the bird source closest to it.
                let level = stage.sources.filter { $0.kind == .bird }
                    .min { simd_distance($0.world, p) < simd_distance($1.world, p) }?.level ?? 0
                let moving = o.flags & UInt32(ASMR_OBJECT_FLAG_MOVING) != 0
                var h = heading[o.id] ?? atan2(-o.y, -o.x)
                if moving, let last = lastBird[o.id] {
                    let v = p - last
                    if simd_length(SIMD2(v.x, v.y)) > 1e-4 { h = atan2(v.y, v.x) }
                } else if !moving {
                    h = atan2(-o.y, -o.x)
                }
                heading[o.id] = h
                lastBird[o.id] = p
                guard let mesh = lib[moving ? "bird_flying" : "bird"] else { continue }
                let wing: Float = moving ? sin(time * 14 + Float(o.id)) * 0.85 : 0
                let m = Self.transform(p, yaw: h, scale: Self.birdScale)
                list.append((mesh, Instance(model: m, info: SIMD4(Float(o.id) + 1000, flags(.object(o.id)), 0, 0), anim: SIMD4(wing, 0, 0, 0))))
                if !moving, !flying, let pole {
                    // A dead snag to perch on, with one twig.
                    let g = ground(o.x, o.y)
                    let top = o.z - 0.1 * Self.birdScale
                    let snag = Self.transform(SIMD3(o.x, o.y, g - 0.2), yaw: 0, scale: 1) * Self.scaleMatrix(SIMD3(0.07, 0.07, top - g + 0.2))
                    let twig = Self.transform(SIMD3(o.x - 0.05, o.y, top - 0.05), yaw: h + 0.4, scale: 1)
                        * simd_float4x4(simd_quatf(angle: 1.2, axis: SIMD3(0, 1, 0))) * Self.scaleMatrix(SIMD3(0.03, 0.03, 0.7))
                    for mm in [snag, twig] {
                        list.append((pole, Instance(model: mm, info: SIMD4(Float(o.id) + 1000, flags(.object(o.id)), 0, 0), anim: .zero)))
                    }
                }
                if birds < 4, let sp = proj.project(p) {
                    let sx: Float = sp.point.x * scale, sy: Float = sp.point.y * scale
                    let perMetre: Float = focal / sp.w * scale
                    data[8 + birds] = SIMD4(sx, sy, perMetre, level)
                    data[12 + birds] = SIMD4(simd_distance(stage.camera.eye, p), 0, 0, 0)
                    birds += 1
                }
            default:
                break
            }
        }
        for (x, y, s, name) in Self.grove where !flying {
            guard let mesh = lib[name] else { continue }
            let wind = localWind(y)
            let m = Self.transform(SIMD3(x, y, Terrain.height(x, y)), yaw: x * 0.7 + y, scale: s) * Self.slender
            list.append((mesh, Instance(model: m, info: SIMD4(-1, 0, wind * 0.55, 6.6), anim: SIMD4(0, x + y, wind, 1))))
        }
        // The forest goes on beyond the play area, thinning out into the haze; none in the river.
        for (x, y, s, name) in Self.distantForest {
            guard let mesh = lib[name], abs(x - Stage.riverAt(y, river).x) > 4 else { continue }
            let m = Self.transform(SIMD3(x, y, Terrain.farHeight(x, y) - 0.1), yaw: x + y * 0.3, scale: s) * Self.slender
            list.append((mesh, Instance(model: m, info: SIMD4(-1, 0, 0, 6.6), anim: SIMD4(0, x, localWind(y), 1))))
        }
        if let herm = lib["herm"] {
            // The listener: the bust turns with the head.
            let m = Self.transform(SIMD3(0, 0, 0), yaw: stage.headPose.yaw, scale: Self.hermScale)
            list.append((herm, Instance(model: m, info: SIMD4(-1, 0, 0, 0), anim: .zero)))
        }
        if !flying, let ruin = lib["ruin"] {
            let r = Self.ruin
            let m = Self.transform(SIMD3(r.x, r.y, Terrain.height(r.x, r.y) - 0.05), yaw: 0.4, scale: 1)
            list.append((ruin, Instance(model: m, info: SIMD4(-1, 0, 0, 0), anim: .zero)))
        }
        // Generic sources, drawn quietly: rain curtains, a campfire, crickets in the grass.
        var generic = 0
        // Fire first, then crickets, then the loudest rain patches: the shader takes eight.
        let order: [SourceInfo.Kind: Int] = [.fire: 0, .insect: 1, .rain: 2]
        let candidates = stage.sources.filter { order[$0.kind] != nil }
            .sorted { (order[$0.kind]!, -$0.level) < (order[$1.kind]!, -$1.level) }
        for s in candidates where generic < 8 {
            if isAbsent(s.world) { continue }
            guard let sp = proj.project(s.world) else { continue }
            if s.kind == .fire, let fire = lib["campfire"] {
                let g = Terrain.height(s.world.x, s.world.y)
                list.append((fire, Instance(model: Self.transform(SIMD3(s.world.x, s.world.y, g), yaw: 0.3, scale: 1),
                                            info: SIMD4(-1, 0, 0, 0), anim: .zero)))
            }
            let base = s.kind == .fire ? (proj.project(SIMD3(s.world.x, s.world.y, Terrain.height(s.world.x, s.world.y) + 0.15)) ?? sp) : sp
            data[32 + generic] = SIMD4(s.world, s.level)
            data[40 + generic] = SIMD4(Float(s.kind.rawValue), base.point.x * scale, base.point.y * scale, focal / base.w * scale)
            generic += 1
        }
        u.counts = SIMD4(Float(min(river.count, 8)), Float(birds), stage.audio.water, Float(generic))
        return list
    }

    /// Whether an engine object near this point is off stage (ASMR_OBJECT_FLAG_ABSENT).
    private func isAbsent(_ p: SIMD3<Float>) -> Bool {
        stage.objects.contains {
            $0.kind == UInt32(ASMR_OBJECT_SOURCE) && $0.flags & UInt32(ASMR_OBJECT_FLAG_ABSENT) != 0
                && simd_distance(SIMD3($0.x, $0.y, $0.z), p) < 0.75
        }
    }

    private func localWind(_ y: Float) -> Float {
        let x = min(max((20 - y) / 40, 0), 1) * 63
        let i0 = Int(x), i1 = min(i0 + 1, 63)
        return stage.wind[i0] + (stage.wind[i1] - stage.wind[i0]) * (x - Float(i0))
    }

    // MARK: Helpers

    private func ensureTargets(width: Int, height: Int) {
        if let gA, gA.width == width, gA.height == height { return }
        func tex(_ f: MTLPixelFormat) -> MTLTexture? {
            let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: f, width: width, height: height, mipmapped: false)
            d.usage = [.renderTarget, .shaderRead]
            d.storageMode = .private
            return GPU.shared.device.makeTexture(descriptor: d)
        }
        gA = tex(.rgba16Float)
        gB = tex(.rgba16Float)
        gDepth = tex(.depth32Float)
    }

    private func snapped(_ b: SIMD4<Float>, scale: Float) -> SIMD4<Float> {
        let pad: Float = 6
        return (SIMD4(b.x - pad, b.y - pad, b.z + pad, b.w + pad) * scale).rounded(.down)
    }

    static func transform(_ t: SIMD3<Float>, yaw: Float, scale: Float) -> simd_float4x4 {
        var m = simd_float4x4(simd_quatf(angle: yaw, axis: SIMD3(0, 0, 1)))
        m.columns.0 *= scale
        m.columns.1 *= scale
        m.columns.2 *= scale
        m.columns.3 = SIMD4(t, 1)
        return m
    }

    /// 1 by day, 0 at night, with a dawn and a dusk between.
    static func daylight(_ hour: Float) -> Float {
        func smooth(_ a: Float, _ b: Float, _ x: Float) -> Float {
            let t = min(max((x - a) / (b - a), 0), 1)
            return t * t * (3 - 2 * t)
        }
        return smooth(4.5, 7.5, hour) * (1 - smooth(18.5, 21.5, hour))
    }

    static func scaleMatrix(_ s: SIMD3<Float>) -> simd_float4x4 {
        simd_float4x4(diagonal: SIMD4(s, 1))
    }

    /// Orthographic view from the sun over the whole disc.
    static func sunMatrix() -> simd_float4x4 {
        let e = sun * 40
        let f = simd_normalize(-e)
        let s = simd_normalize(simd_cross(f, SIMD3(0, 0, 1)))
        let u = simd_cross(s, f)
        let view = simd_float4x4(rows: [
            SIMD4(s.x, s.y, s.z, -simd_dot(s, e)),
            SIMD4(u.x, u.y, u.z, -simd_dot(u, e)),
            SIMD4(-f.x, -f.y, -f.z, simd_dot(f, e)),
            SIMD4(0, 0, 0, 1),
        ])
        let r: Float = 17, near: Float = 1, far: Float = 90
        let ortho = simd_float4x4(rows: [
            SIMD4(1 / r, 0, 0, 0),
            SIMD4(0, 1 / r, 0, 0),
            SIMD4(0, 0, -1 / (far - near), -near / (far - near)),
            SIMD4(0, 0, 0, 1),
        ])
        return ortho * view
    }

    /// A unit cylinder (radius 1, z 0..1), part wood, for snags and twigs.
    private static func makePole() -> Mesh? {
        var v: [MeshVertex] = []
        var idx: [UInt32] = []
        let n = 10
        for i in 0..<n {
            let a = 2 * Float.pi * Float(i) / Float(n)
            let nr = SIMD3(cos(a), sin(a), 0)
            // Tapers towards the top.
            v.append(MeshVertex(SIMD3(cos(a), sin(a), 0), nr, Part.wood))
            v.append(MeshVertex(SIMD3(cos(a) * 0.45, sin(a) * 0.45, 1), nr, Part.wood))
        }
        for i in 0..<n {
            let a = UInt32(2 * i), b = UInt32(2 * ((i + 1) % n))
            idx += [a, b, a + 1, b, b + 1, a + 1]
        }
        return Mesh.make(v, idx)
    }
}
