import AVFoundation
import CAsmr

struct SourceInfo {
    enum Kind: UInt32 { case water = 0, wind = 1, bird = 2 }
    var kind: Kind
    var world: SIMD3<Float>
    var rel: SIMD3<Float>
    var level: Float
    var freq: Float
}

struct Telemetry {
    var dspLoad: Float = 0
    var gust: Float = 0
    var sampleRate: Float = 0
    var bubbleRate: Float = 0
    var outputLevel: Float = 0
}

/// Hosts the Rust engine inside AVAudioEngine. The Rust side does all DSP; this class only
/// moves buffers, forwards parameters and reads telemetry.
final class AudioEngine: ObservableObject {
    @Published private(set) var isPlaying = false
    @Published var volume: Float = 0.7 { didSet { set(ASMR_PARAM_VOLUME, volume) } }
    @Published var wind: Float = 0.45 { didSet { set(ASMR_PARAM_WIND, wind) } }
    @Published var birds: Float = 0.6 { didSet { set(ASMR_PARAM_BIRDS, birds) } }
    @Published var water: Float = 0.6 { didSet { set(ASMR_PARAM_WATER, water) } }
    @Published var orbit: Float = 0 { didSet { set(ASMR_PARAM_ORBIT, orbit) } }

    private let engine = AVAudioEngine()
    private var node: AVAudioSourceNode?
    private var core: OpaquePointer
    private var coreRate: Double = 48_000
    private var headPose: (Float, Float, Float) = (0, 0, 0)
    private var stopWork: DispatchWorkItem?
    private var rawSources = [AsmrSourceInfo](repeating: AsmrSourceInfo(), count: 32)

    init() {
        core = asmr_engine_new(48_000)
        applyParams()
        NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
        ) { [weak self] _ in
            self?.handleConfigurationChange()
        }
    }

    deinit {
        engine.stop()
        asmr_engine_free(core)
    }

    // MARK: Transport

    func play() {
        stopWork?.cancel()
        stopWork = nil
        if !engine.isRunning {
            let hwRate = engine.outputNode.outputFormat(forBus: 0).sampleRate
            let rate = hwRate > 0 ? hwRate : 48_000
            if node == nil || rate != coreRate {
                rebuild(sampleRate: rate)
            }
            do {
                try engine.start()
            } catch {
                NSLog("ASMR X3000: audio engine failed to start: \(error)")
                return
            }
        }
        isPlaying = true
        set(ASMR_PARAM_PLAYING, 1)
    }

    func pause() {
        isPlaying = false
        set(ASMR_PARAM_PLAYING, 0)
        // Let the engine fade out, then release the audio device.
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.isPlaying else { return }
            self.engine.stop()
        }
        stopWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4, execute: work)
    }

    /// Debug: advance the engine without an audio device (used by --snapshot).
    func simulate(seconds: Double) {
        isPlaying = true
        set(ASMR_PARAM_PLAYING, 1)
        let frames = Int(seconds * coreRate)
        var left = [Float](repeating: 0, count: frames)
        var right = [Float](repeating: 0, count: frames)
        left.withUnsafeMutableBufferPointer { l in
            right.withUnsafeMutableBufferPointer { r in
                asmr_render(core, l.baseAddress, r.baseAddress, UInt32(frames))
            }
        }
    }

    func setHeadPose(yaw: Float, pitch: Float, roll: Float) {
        headPose = (yaw, pitch, roll)
        set(ASMR_PARAM_HEAD_YAW, yaw)
        set(ASMR_PARAM_HEAD_PITCH, pitch)
        set(ASMR_PARAM_HEAD_ROLL, roll)
    }

    // MARK: Telemetry (main thread)

    func readTelemetry(spectrum: inout [Float], windField: inout [Float], sources: inout [SourceInfo]) -> Telemetry {
        spectrum.withUnsafeMutableBufferPointer { asmr_get_spectrum(core, $0.baseAddress, UInt32($0.count)) }
        windField.withUnsafeMutableBufferPointer { asmr_get_wind_field(core, $0.baseAddress, UInt32($0.count)) }
        let n = rawSources.withUnsafeMutableBufferPointer {
            asmr_get_sources(core, $0.baseAddress, UInt32($0.count))
        }
        sources.removeAll(keepingCapacity: true)
        for raw in rawSources.prefix(Int(n)) {
            sources.append(SourceInfo(
                kind: SourceInfo.Kind(rawValue: raw.kind) ?? .water,
                world: SIMD3(raw.world_x, raw.world_y, raw.world_z),
                rel: SIMD3(raw.rel_x, raw.rel_y, raw.rel_z),
                level: raw.level,
                freq: raw.freq
            ))
        }
        return Telemetry(
            dspLoad: telemetry(ASMR_TELEMETRY_DSP_LOAD),
            gust: telemetry(ASMR_TELEMETRY_GUST),
            sampleRate: telemetry(ASMR_TELEMETRY_SAMPLE_RATE),
            bubbleRate: telemetry(ASMR_TELEMETRY_BUBBLE_RATE),
            outputLevel: telemetry(ASMR_TELEMETRY_OUTPUT_LEVEL)
        )
    }

    // MARK: Private

    private func set<P: BinaryInteger>(_ param: P, _ value: Float) {
        asmr_set_param(core, UInt32(param), value)
    }

    private func telemetry<P: BinaryInteger>(_ which: P) -> Float {
        asmr_get_telemetry(core, UInt32(which))
    }

    private func applyParams() {
        set(ASMR_PARAM_VOLUME, volume)
        set(ASMR_PARAM_WIND, wind)
        set(ASMR_PARAM_BIRDS, birds)
        set(ASMR_PARAM_WATER, water)
        set(ASMR_PARAM_ORBIT, orbit)
        set(ASMR_PARAM_PLAYING, isPlaying ? 1 : 0)
        setHeadPose(yaw: headPose.0, pitch: headPose.1, roll: headPose.2)
    }

    /// Must only run while the AVAudioEngine is stopped.
    private func rebuild(sampleRate rate: Double) {
        if let old = node {
            engine.detach(old)
            node = nil
        }
        if rate != coreRate {
            let old = core
            core = asmr_engine_new(Float(rate))
            coreRate = rate
            applyParams()
            asmr_engine_free(old)
        }
        guard let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 2) else { return }
        let core = self.core
        let source = AVAudioSourceNode(format: format) { _, _, frameCount, bufferList -> OSStatus in
            let buffers = UnsafeMutableAudioBufferListPointer(bufferList)
            guard buffers.count >= 2,
                  let left = buffers[0].mData?.assumingMemoryBound(to: Float.self),
                  let right = buffers[1].mData?.assumingMemoryBound(to: Float.self)
            else { return noErr }
            asmr_render(core, left, right, frameCount)
            return noErr
        }
        engine.attach(source)
        engine.connect(source, to: engine.mainMixerNode, format: format)
        node = source
    }

    /// Output device or its sample rate changed (e.g. AirPods connected). AVAudioEngine has
    /// already stopped itself; rebuild for the new format.
    private func handleConfigurationChange() {
        let wasPlaying = isPlaying
        engine.stop()
        if let old = node {
            engine.detach(old)
            node = nil
        }
        if wasPlaying {
            play()
        }
    }
}
