import AppKit
import CoreText
import Metal
import simd

/// Short names on the stage for the sounds of a clip scene: the one under the pointer, and the
/// two loudest things sounding now ("humpback", "tingsha"). Lowercase, in the panel's face and
/// its graphite grey, each on a hairline leader from the thing up to a short underline, like a
/// note pencilled on a plan. A name fades in when its sound rises and stays a few seconds after,
/// so the labels do not flicker with the sound. Effects (wind, rain, glows) and sounds with no
/// name of their own get none. Labels keep clear of the scene picker (top left), the settings
/// button (top right), the hint chip (bottom left) and each other.
final class ClipLabels {
    /// A sound that may get a name this frame.
    struct Candidate {
        var id: UInt32
        var name: String
        /// Mapped place and the half size of what is drawn there.
        var point: SIMD3<Float>
        var extent: Float
        /// 0...1 as `SceneRenderer.clipLevel`.
        var level: Float
        var hovered: Bool
    }

    private struct Shown {
        var alpha: Float = 0
        var lastWanted: Float = -100
        var side: Float = 1
    }

    private let pipeline: MTLRenderPipelineState
    private var shown: [UInt32: Shown] = [:]
    private var glyphs: [String: (texture: MTLTexture, size: SIMD2<Float>)] = [:]
    private var lastTime: Float?
    private var firstSeen: [UInt32: Float] = [:]

    /// Seconds a name stays after its sound has dropped.
    static let hold: Float = 3
    static let fontSize: CGFloat = 10.5

    init(format: MTLPixelFormat) {
        let gpu = GPU.shared
        let d = MTLRenderPipelineDescriptor()
        d.vertexFunction = gpu.library.makeFunction(name: "overlayVert")
        d.fragmentFunction = gpu.library.makeFunction(name: "overlayFrag")
        let c = d.colorAttachments[0]!
        c.pixelFormat = format
        c.isBlendingEnabled = true
        c.sourceRGBBlendFactor = .sourceAlpha
        c.destinationRGBBlendFactor = .oneMinusSourceAlpha
        c.sourceAlphaBlendFactor = .one
        c.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        pipeline = try! gpu.device.makeRenderPipelineState(descriptor: d)
    }

    /// Forgets every label (a new scene).
    func reset() {
        shown.removeAll()
        firstSeen.removeAll()
        lastTime = nil
    }

    /// Draws the labels over `target` (sized `size` points at `scale`).
    func encode(_ cb: MTLCommandBuffer, target: MTLTexture, proj: Projector, scale: Float, time: Float,
                candidates: [Candidate]) {
        let dt = min(max(time - (lastTime ?? time), 0), 0.25)
        // The first frame (the popover opening, a snapshot) shows the names at once.
        let first = lastTime == nil
        lastTime = time
        // Wanted: the hovered one, and the two that matter most now: a sound that has just come
        // on stage, then the loudest, then the biggest and nearest.
        for c in candidates where firstSeen[c.id] == nil { firstSeen[c.id] = first ? -100 : time }
        let present = Set(candidates.map(\.id))
        for id in Array(firstSeen.keys) where !present.contains(id) { firstSeen[id] = nil }
        func weight(_ c: Candidate) -> Float {
            let fresh: Float = time - (firstSeen[c.id] ?? -100) < 5 ? 2 : 0
            // A name already up keeps its place against a near tie.
            let up: Float = (shown[c.id]?.alpha ?? 0) > 0.5 ? 0.6 : 0
            return fresh + up + 1.5 * c.level + c.extent / (1 + 0.15 * simd_length(c.point))
        }
        var wanted = Set(candidates.filter(\.hovered).map(\.id))
        // One name once: two bowls struck in turn are not labelled twice.
        var names = Set(candidates.filter(\.hovered).map(\.name))
        for c in candidates.filter({ !$0.hovered }).sorted(by: { weight($0) > weight($1) }) {
            if names.contains(c.name) { continue }
            names.insert(c.name)
            wanted.insert(c.id)
            if wanted.subtracting(candidates.filter(\.hovered).map(\.id)).count >= 2 { break }
        }
        for c in candidates where wanted.contains(c.id) { shown[c.id, default: Shown()].lastWanted = time }
        for id in Array(shown.keys) {
            var s = shown[id]!
            let on = present.contains(id) && time - s.lastWanted < Self.hold
            s.alpha = first && on ? 1 : min(max(s.alpha + (on ? dt / 0.35 : -dt / 0.6), 0), 1)
            if s.alpha <= 0 && !on { shown[id] = nil } else { shown[id] = s }
        }
        // Hovered first, then the loudest: earlier labels take the room.
        let order = candidates.filter { (shown[$0.id]?.alpha ?? 0) > 0.01 }
            .sorted { ($0.hovered ? 2 : 0) + $0.level > ($1.hovered ? 2 : 0) + $1.level }
        guard !order.isEmpty else { return }

        let size = proj.size
        // Where the panel's own controls sit over the stage (points).
        let reserved: [SIMD4<Float>] = [
            SIMD4(0, 0, 170, 44), SIMD4(size.x - 52, 0, size.x, 44), SIMD4(0, size.y - 44, 270, size.y),
        ]
        var taken: [SIMD4<Float>] = []
        var verts: [Float] = []
        var draws: [(texture: MTLTexture?, first: Int, count: Int)] = []
        func overlaps(_ r: SIMD4<Float>) -> Bool {
            if r.x < 4 || r.y < 4 || r.z > size.x - 4 || r.w > size.y - 4 { return true }
            return (reserved + taken).contains { r.x < $0.z && r.z > $0.x && r.y < $0.w && r.w > $0.y }
        }
        func push(_ p: SIMD2<Float>, _ uv: SIMD2<Float>, _ c: SIMD4<Float>) {
            verts += [p.x * scale, p.y * scale, uv.x, uv.y, c.x, c.y, c.z, c.w]
        }
        func quad(_ a: SIMD2<Float>, _ b: SIMD2<Float>, _ cc: SIMD2<Float>, _ d: SIMD2<Float>,
                  uv: (SIMD2<Float>, SIMD2<Float>, SIMD2<Float>, SIMD2<Float>) = (.zero, .zero, .zero, .zero), _ col: SIMD4<Float>) {
            push(a, uv.0, col); push(b, uv.1, col); push(cc, uv.2, col)
            push(a, uv.0, col); push(cc, uv.2, col); push(d, uv.3, col)
        }
        func segment(_ a: SIMD2<Float>, _ b: SIMD2<Float>, width w: Float, _ col: SIMD4<Float>) {
            let d = b - a
            let l = simd_length(d)
            guard l > 0.01 else { return }
            let n = SIMD2(-d.y, d.x) / l * (w / 2)
            quad(a + n, b + n, b - n, a - n, col)
        }

        for c in order {
            guard var state = shown[c.id], let at = proj.project(c.point) else { continue }
            let top = proj.project(c.point + SIMD3(0, 0, min(c.extent * 0.7, 0.8)))?.point.y ?? at.point.y
            let anchor = SIMD2(at.point.x, min(top, at.point.y) - 3)
            guard anchor.x > 0, anchor.x < size.x, anchor.y > 0, anchor.y < size.y + 20 else { continue }
            guard let g = glyph(c.name, scale: scale) else { continue }
            let tw = g.size.x / scale, th = g.size.y / scale
            // Up and to the right, or the other way where that is taken.
            var placed: (elbow: SIMD2<Float>, rect: SIMD4<Float>, side: Float, up: Float)?
            let sides: [Float] = [state.side, -state.side]
            for (up, reach) in [(Float(1), Float(1)), (1, 2), (-1, 1)] {
                for side in sides {
                    let elbow = anchor + SIMD2(10 * side, -12 * up) * reach
                    let x0 = side > 0 ? elbow.x : elbow.x - tw - 4
                    let rect = SIMD4(x0, elbow.y - th - 1, x0 + tw + 4, elbow.y + 2)
                    if !overlaps(rect) { placed = (elbow, rect, side, up); break }
                }
                if placed != nil { break }
            }
            guard let p = placed else { continue }
            state.side = p.side
            shown[c.id] = state
            taken.append(p.rect)
            let a = state.alpha
            let grey: Float = c.hovered ? 0.93 : 0.62
            let ink = SIMD4<Float>(grey, grey, grey, a)
            let line = SIMD4<Float>(0.62, 0.62, 0.62, a * 0.7)
            let first = verts.count / 8
            // The node on the thing, the leader up to the elbow, the underline.
            let dot: Float = 1.5
            quad(anchor + SIMD2(-dot, -dot), anchor + SIMD2(dot, -dot), anchor + SIMD2(dot, dot), anchor + SIMD2(-dot, dot), line)
            segment(anchor, p.elbow, width: 1 / scale * 1.2, line)
            let under = p.elbow + SIMD2((tw + 4) * p.side, 0)
            segment(p.elbow, under, width: 1 / scale * 1.2, line)
            draws.append((nil, first, verts.count / 8 - first))
            let tFirst = verts.count / 8
            let o = SIMD2(p.rect.x + 2, p.rect.y)
            // A soft dark edge under the letters keeps them legible over lines and the water.
            let shade = SIMD4<Float>(0, 0, 0, a * 0.5)
            for d in [SIMD2<Float>(0.7, 0), SIMD2(-0.7, 0), SIMD2(0, 0.7), SIMD2(0, -0.7)] {
                let q = o + d
                quad(q, q + SIMD2(tw, 0), q + SIMD2(tw, th), q + SIMD2(0, th),
                     uv: (SIMD2(0, 0), SIMD2(1, 0), SIMD2(1, 1), SIMD2(0, 1)), shade)
            }
            quad(o, o + SIMD2(tw, 0), o + SIMD2(tw, th), o + SIMD2(0, th),
                 uv: (SIMD2(0, 0), SIMD2(1, 0), SIMD2(1, 1), SIMD2(0, 1)), ink)
            draws.append((g.texture, tFirst, 30))
        }
        guard !verts.isEmpty, let buf = GPU.shared.device.makeBuffer(bytes: verts, length: verts.count * 4, options: .storageModeShared) else { return }
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = target
        pass.colorAttachments[0].loadAction = .load
        pass.colorAttachments[0].storeAction = .store
        guard let enc = cb.makeRenderCommandEncoder(descriptor: pass) else { return }
        enc.setRenderPipelineState(pipeline)
        enc.setVertexBuffer(buf, offset: 0, index: 0)
        var px = SIMD2(Float(target.width), Float(target.height))
        enc.setVertexBytes(&px, length: 8, index: 1)
        // Lines sample nothing, but the texture slot is never left empty.
        let any = draws.compactMap(\.texture).first
        for d in draws {
            var textured: Float = d.texture == nil ? 0 : 1
            enc.setFragmentBytes(&textured, length: 4, index: 0)
            if let t = d.texture ?? any { enc.setFragmentTexture(t, index: 0) }
            enc.drawPrimitives(type: .triangle, vertexStart: d.first, vertexCount: d.count)
        }
        enc.endEncoding()
    }

    /// The name set in Coppice Mono as a coverage texture, made once per name and scale.
    private func glyph(_ text: String, scale: Float) -> (texture: MTLTexture, size: SIMD2<Float>)? {
        let key = "\(scale)|\(text)"
        if let g = glyphs[key] { return g }
        if glyphs.count > 96 { glyphs.removeAll() }
        let size = Self.fontSize * CGFloat(scale)
        let font = CTFontCreateWithName("CoppiceMono-Regular" as CFString, size, nil)
        let attr = NSAttributedString(string: text, attributes: [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ])
        let line = CTLineCreateWithAttributedString(attr)
        var ascent: CGFloat = 0, descent: CGFloat = 0, leading: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        let w = Int(width.rounded(.up)) + 2, h = Int((ascent + descent).rounded(.up)) + 2
        guard w > 2, let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w,
                                         space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return nil }
        ctx.setFillColor(gray: 0, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
        ctx.setFillColor(gray: 1, alpha: 1)
        ctx.setShouldSmoothFonts(false)
        ctx.textPosition = CGPoint(x: 1, y: descent + 1)
        CTLineDraw(line, ctx)
        guard let data = ctx.data else { return nil }
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .r8Unorm, width: w, height: h, mipmapped: false)
        d.usage = .shaderRead
        guard let tex = GPU.shared.device.makeTexture(descriptor: d) else { return nil }
        // A bitmap context's first row is the top of the picture, as in the texture.
        tex.replace(region: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0, withBytes: data, bytesPerRow: w)
        let g = (tex, SIMD2(Float(w), Float(h)))
        glyphs[key] = g
        return g
    }
}
