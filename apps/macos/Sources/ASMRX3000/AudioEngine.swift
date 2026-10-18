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

/// One slider of the engine's control tree (asmr_get_controls).
struct Control: Equatable {
    var id: UInt32
    var group: UInt32
    var parent: UInt32?
    var flags: UInt32
    var min: Float
    var max: Float
    var value: Float
    var defaultValue: Float
    var key: String
    var unit: String

    var isMacro: Bool { flags & UInt32(ASMR_CONTROL_FLAG_MACRO) != 0 }
    var isLog: Bool { flags & UInt32(ASMR_CONTROL_FLAG_LOG) != 0 && min > 0 }
    var isDiscrete: Bool { flags & UInt32(ASMR_CONTROL_FLAG_DISCRETE) != 0 }

    /// Position along the slider, 0..1.
    var fraction: Float {
        guard max > min else { return 0 }
        if isLog { return log(value / min) / log(max / min) }
        return (value - min) / (max - min)
    }

    func value(atFraction f: Float) -> Float {
        let f = Swift.min(Swift.max(f, 0), 1)
        if isLog { return min * pow(max / min, f) }
        let v = min + (max - min) * f
        return isDiscrete ? v.rounded() : v
    }
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
    private var rawObjects = [AsmrObjectInfo](repeating: AsmrObjectInfo(), count: 32)
    private var rawControls = [AsmrControlInfo](repeating: AsmrControlInfo(), count: 64)
    private var controlNames: [UInt32: (String, String)] = [:]
    /// Control values the user set, in order, re-applied when the core is rebuilt.
    private var setControls: [(UInt32, Float)] = []
    /// Positions the user gave objects; re-applied when the core is rebuilt for a new sample rate.
    private var placed: [UInt32: SIMD3<Float>] = [:]

    /// Binaural renderer. `core`: our MagLS decoder of the measured SADIE II HRTF (default).
    /// `apple`: the engine renders a 7.1.4 bed and Apple's AUSpatialMixer makes it binaural
    /// with the listener's personalised HRTF when the system grants it, head rotation still
    /// ours (predicted). `appleTracked`: same, but Apple applies AirPods head tracking itself.
    enum Renderer: String, CaseIterable { case core, apple, appleTracked }
    @Published var renderer: Renderer = Renderer(rawValue: UserDefaults.standard.string(forKey: "renderer") ?? "") ?? .core {
        didSet {
            UserDefaults.standard.set(renderer.rawValue, forKey: "renderer")
            if renderer != oldValue { switchRenderer() }
        }
    }
    /// Whether AUSpatialMixer reports that it renders with a personalised HRTF.
    @Published private(set) var personalizedHRTF = false
    /// Render-to-ear latency the head pose is predicted over (s), and how it was derived.
    @Published private(set) var outputLatency: Double = 0
    @Published private(set) var outputDescription = ""

    /// Guards `core` against the motion queue while `rebuild` swaps engines.
    private let coreLock = NSLock()
    private var lastHead: HeadTracker.Sample?
    private var spatialMixer: AVAudioUnit?
    private var spatialMixerPending = false
    private let bedPointers = UnsafeMutablePointer<UnsafeMutablePointer<Float>?>.allocate(capacity: 12)

    init() {
        core = asmr_engine_new(48_000)
        applyParams()
        if renderer != .core { prepareSpatialMixer() }
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
            updateOutputLatency()
            updatePersonalizedHRTF()
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

    /// Head pose from the tracker, called on its motion queue (the engine side is lock-free).
    func setHeadSample(_ s: HeadTracker.Sample) {
        coreLock.lock()
        defer { coreLock.unlock() }
        lastHead = s
        asmr_set_head_pose(core, s.q.real, s.q.imag.x, s.q.imag.y, s.q.imag.z, s.w.x, s.w.y, s.w.z, s.age)
    }

    /// Legacy Euler pose (used only until the first quaternion sample).
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

    /// Scene objects (id, kind, position, flags). Ids are stable for the engine's lifetime.
    func readObjects(_ out: inout [AsmrObjectInfo]) {
        let n = rawObjects.withUnsafeMutableBufferPointer {
            asmr_get_objects(core, $0.baseAddress, UInt32($0.count))
        }
        out.removeAll(keepingCapacity: true)
        out.append(contentsOf: rawObjects.prefix(Int(n)))
    }

    /// The control tree: macros and their children. Values change when a macro moves, so this is
    /// read every frame.
    func readControls(_ out: inout [Control]) {
        let n = rawControls.withUnsafeMutableBufferPointer {
            asmr_get_controls(core, $0.baseAddress, UInt32($0.count))
        }
        out.removeAll(keepingCapacity: true)
        for var raw in rawControls.prefix(Int(n)) {
            let names: (String, String)
            if let cached = controlNames[raw.id] {
                names = cached
            } else {
                let key = withUnsafeBytes(of: &raw.key) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
                let unit = withUnsafeBytes(of: &raw.unit) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
                names = (key, unit)
                controlNames[raw.id] = names
            }
            out.append(Control(id: raw.id, group: raw.group, parent: raw.parent == UInt32.max ? nil : raw.parent,
                               flags: raw.flags, min: raw.min, max: raw.max, value: raw.value,
                               defaultValue: raw.default_value, key: names.0, unit: names.1))
        }
    }

    func setControl(_ id: UInt32, _ value: Float) {
        setControls.removeAll { $0.0 == id }
        setControls.append((id, value))
        asmr_set_control(core, id, value)
    }

    /// Live peak waveform of a track (ASMR_SCOPE_*), oldest first, one point per 64 samples.
    func readScope(_ track: Int, into out: inout [Float]) {
        out.withUnsafeMutableBufferPointer { _ = asmr_get_scope(core, UInt32(track), $0.baseAddress, UInt32($0.count)) }
    }

    func setObjectPosition(id: UInt32, _ p: SIMD3<Float>) {
        placed[id] = p
        asmr_set_object_position(core, id, p.x, p.y, p.z)
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
        if let s = lastHead {
            asmr_set_head_pose(core, s.q.real, s.q.imag.x, s.q.imag.y, s.q.imag.z, s.w.x, s.w.y, s.w.z, s.age)
        }
        if outputLatency > 0 { set(ASMR_PARAM_OUTPUT_LATENCY, Float(outputLatency)) }
        // Listening-check overrides (docs/spatial-engine.md 6.5).
        let d = UserDefaults.standard
        if d.object(forKey: "headPrediction") != nil { set(ASMR_PARAM_HEAD_PREDICTION, d.float(forKey: "headPrediction")) }
        if d.object(forKey: "headParallax") != nil { set(ASMR_PARAM_HEAD_PARALLAX, d.float(forKey: "headParallax")) }
        for (id, p) in placed {
            asmr_set_object_position(core, id, p.x, p.y, p.z)
        }
        for (id, v) in setControls {
            asmr_set_control(core, id, v)
        }
    }

    /// Must only run while the AVAudioEngine is stopped.
    private func rebuild(sampleRate rate: Double) {
        if let old = node {
            engine.detach(old)
            node = nil
        }
        if rate != coreRate {
            coreLock.lock()
            let old = core
            core = asmr_engine_new(Float(rate))
            coreRate = rate
            coreLock.unlock()
            applyParams()
            asmr_engine_free(old)
        }
        if let mixer = spatialMixer, engine.attachedNodes.contains(mixer) {
            engine.detach(mixer)
        }
        if renderer != .core, let mixer = spatialMixer, attachBed(rate: rate, mixer: mixer) {
            return
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

    // MARK: Apple renderer (AUSpatialMixer, personalised HRTF)

    private func switchRenderer() {
        if renderer != .core, spatialMixer == nil {
            prepareSpatialMixer() // rebuilds once the unit exists
            return
        }
        restartGraph()
    }

    private func restartGraph() {
        let wasRunning = engine.isRunning
        engine.stop()
        if let old = node {
            engine.detach(old)
            node = nil
        }
        if wasRunning || isPlaying { play() }
    }

    /// AUSpatialMixer has to be instantiated asynchronously.
    private func prepareSpatialMixer() {
        guard !spatialMixerPending else { return }
        spatialMixerPending = true
        let desc = AudioComponentDescription(
            componentType: kAudioUnitType_Mixer,
            componentSubType: kAudioUnitSubType_SpatialMixer,
            componentManufacturer: kAudioUnitManufacturer_Apple,
            componentFlags: 0,
            componentFlagsMask: 0
        )
        AVAudioUnit.instantiate(with: desc, options: []) { [weak self] unit, error in
            DispatchQueue.main.async {
                guard let self else { return }
                self.spatialMixerPending = false
                guard let unit else {
                    NSLog("ASMR X3000: AUSpatialMixer unavailable (\(String(describing: error))), using the core renderer")
                    return
                }
                self.spatialMixer = unit
                if self.renderer != .core { self.restartGraph() }
            }
        }
    }

    /// source (7.1.4 bed from the core) -> AUSpatialMixer (binaural, personalised HRTF) -> main mixer.
    private func attachBed(rate: Double, mixer: AVAudioUnit) -> Bool {
        guard let layout = AVAudioChannelLayout(layoutTag: kAudioChannelLayoutTag_Atmos_7_1_4) else { return false }
        let bedFormat = AVAudioFormat(standardFormatWithSampleRate: rate, channelLayout: layout)
        guard let stereo = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 2) else { return false }
        let au = mixer.audioUnit
        func set(_ id: AudioUnitPropertyID, _ scope: AudioUnitScope, _ value: UInt32) {
            var v = value
            let status = AudioUnitSetProperty(au, id, scope, 0, &v, UInt32(MemoryLayout<UInt32>.size))
            if status != noErr { NSLog("ASMR X3000: AUSpatialMixer property \(id) failed: \(status)") }
        }
        set(kAudioUnitProperty_ElementCount, kAudioUnitScope_Input, 1)
        set(kAudioUnitProperty_SpatializationAlgorithm, kAudioUnitScope_Input, AUSpatializationAlgorithm.spatializationAlgorithm_UseOutputType.rawValue)
        set(kAudioUnitProperty_SpatialMixerSourceMode, kAudioUnitScope_Input, AUSpatialMixerSourceMode.spatialMixerSourceMode_AmbienceBed.rawValue)
        set(kAudioUnitProperty_SpatialMixerOutputType, kAudioUnitScope_Global, AUSpatialMixerOutputType.spatialMixerOutputType_Headphones.rawValue)
        set(kAudioUnitProperty_UsesInternalReverb, kAudioUnitScope_Global, 0)
        set(kAudioUnitProperty_SpatialMixerPersonalizedHRTFMode, kAudioUnitScope_Global, AUSpatialMixerPersonalizedHRTFMode.auto.rawValue)
        set(kAudioUnitProperty_SpatialMixerEnableHeadTracking, kAudioUnitScope_Global, renderer == .appleTracked ? 1 : 0)

        let core = self.core
        let worldLocked: UInt32 = renderer == .appleTracked ? 1 : 0
        // Channel-pointer table allocated here, not in the render callback (no allocation there).
        let ptrs = bedPointers
        let source = AVAudioSourceNode(format: bedFormat) { _, _, frameCount, bufferList -> OSStatus in
            let buffers = UnsafeMutableAudioBufferListPointer(bufferList)
            guard buffers.count == 12 else { return noErr }
            for i in 0..<12 {
                guard let p = buffers[i].mData?.assumingMemoryBound(to: Float.self) else { return noErr }
                ptrs[i] = p
            }
            asmr_render_bed(core, ptrs, 12, frameCount, worldLocked)
            return noErr
        }
        engine.attach(source)
        engine.attach(mixer)
        engine.connect(source, to: mixer, format: bedFormat)
        engine.connect(mixer, to: engine.mainMixerNode, format: stereo)
        node = source
        return true
    }

    private func updatePersonalizedHRTF() {
        guard renderer != .core, let mixer = spatialMixer, engine.attachedNodes.contains(mixer) else {
            personalizedHRTF = false
            return
        }
        var v: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        let status = AudioUnitGetProperty(mixer.audioUnit, kAudioUnitProperty_SpatialMixerAnyInputIsUsingPersonalizedHRTF, kAudioUnitScope_Global, 0, &v, &size)
        personalizedHRTF = status == noErr && v != 0
        NSLog("ASMR X3000: Apple renderer (\(renderer.rawValue)), personalised HRTF: \(personalizedHRTF ? "yes" : "no")")
    }

    // MARK: Output latency (drives head-pose prediction)

    /// Render-to-ear delay: device presentation latency (+ safety offset, stream latency), the IO
    /// buffer, our decoder block, the spatial mixer's own latency, and for Bluetooth the AAC
    /// encoder + radio, which macOS under-reports (assume >= 160 ms in total on Bluetooth).
    /// UserDefaults "headLatencyMs" overrides it (the listening check tunes it).
    private func updateOutputLatency() {
        let out = engine.outputNode
        let rate = max(out.outputFormat(forBus: 0).sampleRate, 8_000)
        var latency = out.presentationLatency
        let device = Self.defaultOutputDevice()
        let bufferFrames = Self.property(device, kAudioDevicePropertyBufferFrameSize, UInt32(0))
        latency += Double(bufferFrames) / rate
        latency += 64.0 / rate
        if renderer != .core, let mixer = spatialMixer { latency += mixer.auAudioUnit.latency }
        let transport = Self.property(device, kAudioDevicePropertyTransportType, UInt32(0))
        let bluetooth = transport == kAudioDeviceTransportTypeBluetooth || transport == kAudioDeviceTransportTypeBluetoothLE
        if bluetooth { latency = max(latency, 0.16) }
        let override = UserDefaults.standard.double(forKey: "headLatencyMs")
        if override > 0 { latency = override / 1000 }
        outputLatency = latency
        outputDescription = String(format: "%@ %.0f Hz, %d-frame buffer, latency %.0f ms%@",
                                   bluetooth ? "Bluetooth" : "wired/built-in", rate, Int(bufferFrames), latency * 1000,
                                   override > 0 ? " (override)" : "")
        NSLog("ASMR X3000: output \(outputDescription)")
        set(ASMR_PARAM_OUTPUT_LATENCY, Float(latency))
    }

    private static func defaultOutputDevice() -> AudioObjectID {
        var addr = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultOutputDevice,
                                              mScope: kAudioObjectPropertyScopeGlobal,
                                              mElement: kAudioObjectPropertyElementMain)
        var id = AudioObjectID(0)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        _ = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size, &id)
        return id
    }

    private static func property(_ device: AudioObjectID, _ selector: AudioObjectPropertySelector, _ fallback: UInt32) -> UInt32 {
        var addr = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal,
                                              mElement: kAudioObjectPropertyElementMain)
        var v = fallback
        var size = UInt32(MemoryLayout<UInt32>.size)
        return AudioObjectGetPropertyData(device, &addr, 0, nil, &size, &v) == noErr ? v : fallback
    }
}
