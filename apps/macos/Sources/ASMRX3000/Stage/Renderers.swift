import CAsmr
import CoreGraphics
import Metal
import simd

/// One device, queue and runtime-compiled library for the whole app.
final class GPU {
    static let shared = GPU()

    let device: MTLDevice
    let queue: MTLCommandQueue
    let library: MTLLibrary

    private init() {
        device = MTLCreateSystemDefaultDevice()!
        queue = device.makeCommandQueue()!
        do {
            var source = Shaders.source
            if ProcessInfo.processInfo.environment["ASMR_DEBUG"] == "1" {
                source = source.replacingOccurrences(of: "constant bool DEBUG_VIEW = false", with: "constant bool DEBUG_VIEW = true")
            }
            library = try device.makeLibrary(source: source, options: nil)
        } catch {
            fatalError("ASMR X3000: shader compile failed: \(error)")
        }
    }

    func pipeline(fragment: String, format: MTLPixelFormat) -> MTLRenderPipelineState {
        let d = MTLRenderPipelineDescriptor()
        d.vertexFunction = library.makeFunction(name: "fullscreen")
        d.fragmentFunction = library.makeFunction(name: fragment)
        d.colorAttachments[0].pixelFormat = format
        return try! device.makeRenderPipelineState(descriptor: d)
    }

    /// Renders offscreen and reads the result back (snapshot mode).
    func image(width: Int, height: Int, encode: (MTLCommandBuffer, MTLTexture) -> Void) -> CGImage? {
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .bgra8Unorm, width: width, height: height, mipmapped: false)
        d.usage = [.renderTarget, .shaderRead]
        d.storageMode = .shared
        guard let tex = device.makeTexture(descriptor: d), let cb = queue.makeCommandBuffer() else { return nil }
        encode(cb, tex)
        cb.commit()
        cb.waitUntilCompleted()
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        tex.getBytes(&bytes, bytesPerRow: width * 4, from: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0)
        guard let provider = CGDataProvider(data: Data(bytes) as CFData) else { return nil }
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                       space: CGColorSpace(name: CGColorSpace.sRGB)!,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }

    static func fullscreenPass(_ cb: MTLCommandBuffer, target: MTLTexture, pipeline: MTLRenderPipelineState,
                               _ bind: (MTLRenderCommandEncoder) -> Void) {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = target
        pass.colorAttachments[0].loadAction = .dontCare
        pass.colorAttachments[0].storeAction = .store
        guard let enc = cb.makeRenderCommandEncoder(descriptor: pass) else { return }
        enc.setRenderPipelineState(pipeline)
        bind(enc)
        enc.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        enc.endEncoding()
    }
}

// MARK: - Spectrogram

private struct SpecUniforms {
    var size: SIMD4<Float>
    var ring: SIMD4<Float>
    var zoom: SIMD4<Float>
    var hover: SIMD4<Float>
    var birds: SIMD4<Float>
    var sel: SIMD4<Float>
}

final class SpectrogramRenderer {
    /// Columns shown across the view (the rest of the history is kept for later zoom-out).
    static let visibleColumns = 600

    private let stage: Stage
    private let pipeline: MTLRenderPipelineState
    private let history: MTLTexture
    private var uploaded = 0

    /// Pointer position in points, nil when outside.
    var hover: SIMD2<Float>?

    init(stage: Stage, format: MTLPixelFormat = .bgra8Unorm) {
        self.stage = stage
        pipeline = GPU.shared.pipeline(fragment: "spectrogram", format: format)
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .r32Float, width: Stage.columns, height: Stage.bands, mipmapped: false)
        d.usage = .shaderRead
        history = GPU.shared.device.makeTexture(descriptor: d)!
    }

    func encode(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float) {
        upload()
        var birds = SIMD4<Float>(repeating: 0)
        var sel: Float = 0
        var n = 0
        for (i, s) in stage.sources.enumerated() where s.kind == .bird && n < 4 {
            if s.freq > 0 && s.level > 0.004 { birds[n] = s.freq }
            if case .object(let id) = stage.selection, id == UInt32(i), s.freq > 0 { sel = s.freq }
            n += 1
        }
        var u = SpecUniforms(
            size: SIMD4(Float(target.width), Float(target.height), scale, Float(stage.time)),
            ring: SIMD4(Float(stage.written - 1), Float(Stage.columns), Float(Stage.bands), Float(Self.visibleColumns)),
            zoom: SIMD4(stage.zoom.x, stage.zoom.y, Stage.fLo, Stage.fHi),
            hover: hover.map { SIMD4($0.x * scale, $0.y * scale, 1, stage.frozen ? 1 : 0) } ?? .zero,
            birds: birds, sel: SIMD4(sel, 0, 0, 0)
        )
        GPU.fullscreenPass(cb, target: target, pipeline: pipeline) { enc in
            enc.setFragmentBytes(&u, length: MemoryLayout<SpecUniforms>.stride, index: 0)
            enc.setFragmentTexture(history, index: 0)
        }
    }

    private func upload() {
        let start = max(uploaded, stage.written - Stage.columns)
        guard start < stage.written else { return }
        stage.history.withUnsafeBytes { raw in
            for c in start..<stage.written {
                let col = c % Stage.columns
                history.replace(region: MTLRegionMake2D(col, 0, 1, Stage.bands), mipmapLevel: 0,
                                withBytes: raw.baseAddress! + col * Stage.bands * 4, bytesPerRow: 4)
            }
        }
        uploaded = stage.written
    }
}

// MARK: - Track scopes

private struct TrackUniforms {
    var size: SIMD4<Float>
    var values: SIMD4<Float>
    var values2: SIMD4<Float>
}

final class TracksRenderer {
    private let stage: Stage
    private let pipeline: MTLRenderPipelineState
    private let scope: MTLTexture

    init(stage: Stage, format: MTLPixelFormat = .bgra8Unorm) {
        self.stage = stage
        pipeline = GPU.shared.pipeline(fragment: "tracks", format: format)
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .r32Float, width: Stage.scopeColumns,
                                                         height: Track.allCases.count, mipmapped: false)
        d.usage = .shaderRead
        scope = GPU.shared.device.makeTexture(descriptor: d)!
    }

    func encode(_ cb: MTLCommandBuffer, target: MTLTexture, size: SIMD2<Float>, scale: Float, values: [Float]) {
        stage.scopes.withUnsafeBytes {
            scope.replace(region: MTLRegionMake2D(0, 0, Stage.scopeColumns, Track.allCases.count), mipmapLevel: 0,
                          withBytes: $0.baseAddress!, bytesPerRow: Stage.scopeColumns * 4)
        }
        func v(_ i: Int) -> Float { i < values.count ? values[i] : 0 }
        var u = TrackUniforms(size: SIMD4(Float(target.width), Float(target.height), scale, Float(Track.allCases.count)),
                              values: SIMD4(v(0), v(1), v(2), v(3)), values2: SIMD4(v(4), 0, 0, 0))
        GPU.fullscreenPass(cb, target: target, pipeline: pipeline) { enc in
            enc.setFragmentBytes(&u, length: MemoryLayout<TrackUniforms>.stride, index: 0)
            enc.setFragmentTexture(scope, index: 0)
        }
    }
}
