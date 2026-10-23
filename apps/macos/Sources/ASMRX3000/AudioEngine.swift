import AVFoundation
import CAsmr

struct SourceInfo {
    enum Kind: UInt32 { case water = 0, wind = 1, bird = 2, rain = 3, thunder = 4, fire = 5, insect = 6 }
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
    /// Names of a discrete control's presets (asmr_get_control_option), index = value.
    var options: [String] = []

    var isMacro: Bool { flags & UInt32(ASMR_CONTROL_FLAG_MACRO) != 0 }
    var isLog: Bool { flags & UInt32(ASMR_CONTROL_FLAG_LOG) != 0 && min > 0 }
    var isDiscrete: Bool { flags & UInt32(ASMR_CONTROL_FLAG_DISCRETE) != 0 }
    /// A child the user set; it no longer follows its macro.
    var isOverridden: Bool { flags & UInt32(ASMR_CONTROL_FLAG_OVERRIDDEN) != 0 }

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
    /// Lightning flash 0..1: the flicker of the leader and return strokes, dimmer with distance.
    var lightning: Float = 0
    /// Leaf and ground wetness 0..1.
    var wetness: Float = 0
    /// Flight mode: cross-fade 0..1, ears above the ground (m), distance travelled (m), turn rate.
    var flight: Float = 0
    var altitude: Float = 1.7
    var odometer: Float = 0
    var speed: Float = 0
    var heading: Float = 0
    var worldX: Float = 0
    var worldY: Float = 0
    /// Sound director: short-term loudness (LUFS, before volume), dynamics gain (dB),
    /// foreground soloists and how masked they are (0..1).
    var loudness: Float = -70
    var mixGain: Float = 0
    var soloists: Float = 0
    var masking: Float = 0
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
    /// Where the listener is (ASMR_PARAM_FLIGHT): home (the forest by the river), flying through
    /// an endless generated landscape (speed: flight.speed), or standing in it (walk with WASD).
    enum Place: Int { case home = 0, flying = 1, standing = 2 }
    @Published var place: Place = .home { didSet { set(ASMR_PARAM_FLIGHT, Float(place.rawValue)) } }
    /// In the generated world, flying or standing (flight levers, streamed objects). Setting it
    /// takes off (true) or goes home (false).
    var flying: Bool {
        get { place != .home }
        set { if newValue != flying { place = newValue ? .flying : .home } }
    }
    /// F: in the world, stop here / fly on; at home, take off.
    func toggleFlight() { place = place == .flying ? .standing : .flying }

    /// Movement keys held in the scene (ASMR_PARAM_MOVE_*): forward (W +1, S -1), strafe
    /// (D +1, A -1), turn (E +1, Q -1), run (Shift).
    func setMoves(forward: Float, strafe: Float, turn: Float, run: Bool) {
        set(ASMR_PARAM_MOVE_FORWARD, forward)
        set(ASMR_PARAM_MOVE_STRAFE, strafe)
        set(ASMR_PARAM_MOVE_TURN, turn)
        set(ASMR_PARAM_MOVE_RUN, run ? 1 : 0)
    }

    /// The screen in front of the listener (ASMR_PARAM_SCREEN_*), metres: width, height and the
    /// distance from the eyes to its centre. Set from ScreenGeometry at launch, on display changes
    /// and by onboarding; kept here so a rebuilt core gets them too.
    private(set) var screen: (width: Float, height: Float, distance: Float)?

    // MARK: Scene

    /// The scene the core runs (SceneEntry.all, or a `--scene path` file for development).
    @Published private(set) var scene: SceneInfo
    /// The scene being built in the background after a switch (nil when none is).
    @Published private(set) var loadingScene: SceneEntry?
    /// Set when the last switch failed (a missing sound file, say); the old scene keeps playing.
    @Published private(set) var sceneError: String?

    /// Clip scenes (ASMR_PARAM_SCENE_*), 1 = as authored. Kept across scene switches and rebuilds.
    enum SceneMacro: Int, CaseIterable {
        case motion, events, bed, creatures
        var param: Int {
            switch self {
            case .motion: return ASMR_PARAM_SCENE_MOTION
            case .events: return ASMR_PARAM_SCENE_EVENTS
            case .bed: return ASMR_PARAM_SCENE_BED
            case .creatures: return ASMR_PARAM_SCENE_CREATURES
            }
        }
        var key: String { ["scene.motion", "scene.events", "scene.bed", "scene.creatures"][rawValue] }
        var max: Float { self == .motion || self == .events ? 3 : 2 }
        /// Stands in for a control id in the panel (the core's ids stay below 1000).
        var controlID: UInt32 { AudioEngine.sceneControlBase + UInt32(rawValue) }
    }
    static let sceneControlBase: UInt32 = 0xF000
    /// Control group of the clip-scene levers (not an ASMR_GROUP_*; the panel and the track
    /// scopes key on it).
    static let sceneGroup: UInt32 = 100
    private var sceneMacros: [Float] = [1, 1, 1, 1]

    func sceneMacro(_ m: SceneMacro) -> Float { sceneMacros[m.rawValue] }

    /// The screen as the core has it (its defaults until ScreenWatcher reports the display).
    var screenSize: (width: Float, height: Float, distance: Float) { screen ?? (0.30, 0.20, 0.60) }

    /// The clip-scene levers as controls, so the panel draws them like the forest's.
    var sceneControls: [Control] {
        SceneMacro.allCases.map { m in
            Control(id: m.controlID, group: Self.sceneGroup + UInt32(m.rawValue), parent: nil,
                    flags: UInt32(ASMR_CONTROL_FLAG_MACRO), min: 0, max: m.max, value: sceneMacros[m.rawValue],
                    defaultValue: 1, key: m.key, unit: "x")
        }
    }

    // MARK: Clip groups

    /// One group of a clip scene's clips ("whales", "current"...): its own level, events, motion,
    /// distance, pitch and, for procedural sounds, model parameters (asmr_set_clip_group_param).
    struct ClipGroup {
        var index: Int
        var id: String
        var title: String
        var icon: String
        /// Bit p = ASMR_CLIP_* parameter p applies.
        var params: UInt32
        var model: UInt32
        /// ASMR_SCOPE_* the group is metered on.
        var scope: Int
        var ranges: [(min: Float, max: Float, def: Float)]
        func has(_ p: Int) -> Bool { params & (1 << UInt32(p)) != 0 }
    }
    @Published private(set) var clipGroups: [ClipGroup] = []
    /// Values per group and parameter; kept per scene in UserDefaults and re-applied to every core.
    private var clipValues: [[Float]] = []
    /// Stands in for control ids in the panel: base + 8 * group + parameter.
    static let clipControlBase: UInt32 = 0xF100

    private func clipDefaultsKey(_ g: ClipGroup, _ p: Int) -> String { "clip.\(scene.entry.id).\(g.id).\(p)" }

    /// Reads the groups of the current core and pushes the remembered values into it.
    private func refreshClipGroups() {
        guard scene.isClip else {
            clipGroups = []
            clipValues = []
            return
        }
        func text(_ call: (UnsafeMutablePointer<CChar>?, UInt32) -> UInt32) -> String {
            var buf = [CChar](repeating: 0, count: 128)
            _ = buf.withUnsafeMutableBufferPointer { call($0.baseAddress, 128) }
            return String(cString: buf)
        }
        var groups: [ClipGroup] = []
        var values: [[Float]] = []
        let d = UserDefaults.standard
        for i in 0..<Int(asmr_clip_group_count(core)) {
            let idx = UInt32(i)
            var info = AsmrClipGroupInfo()
            asmr_clip_group_info(core, idx, &info)
            var ranges: [(min: Float, max: Float, def: Float)] = []
            for p in 0..<7 {
                var r = AsmrClipParamRange()
                asmr_clip_group_param_range(core, idx, UInt32(p), &r)
                ranges.append((r.min, r.max, r.def))
            }
            let g = ClipGroup(index: i, id: text { asmr_clip_group_name(core, idx, $0, $1) },
                              title: text { asmr_clip_group_title(core, idx, $0, $1) },
                              icon: text { asmr_clip_group_icon(core, idx, $0, $1) },
                              params: info.params, model: info.model, scope: Int(info.scope), ranges: ranges)
            var v: [Float] = []
            for p in 0..<7 {
                let key = clipDefaultsKey(g, p)
                let r = ranges[p]
                let stored = d.object(forKey: key) != nil ? d.float(forKey: key) : r.def
                v.append(min(max(stored, r.min), r.max))
                if g.has(p) { asmr_set_clip_group_param(core, idx, UInt32(p), v[p]) }
            }
            ClipGroupLabels.titles[g.id] = g.title
            ClipGroupLabels.icons[g.id] = g.icon
            groups.append(g)
            values.append(v)
        }
        clipGroups = groups
        clipValues = values
    }

    /// Group `i`'s panel number: carries the track its waveform is drawn from (Track.init).
    private static func clipGroupNumber(_ g: ClipGroup) -> UInt32 { 200 + UInt32(g.scope) * 16 + UInt32(min(g.index, 15)) }

    /// Leaf of the control key for parameter `p`: it names the control in the panel.
    private func clipLeaf(_ g: ClipGroup, _ p: Int) -> String {
        switch p {
        case 0: return "level"
        case 1: return "events"
        case 2: return "motion"
        case 3: return "distance"
        case 4: return "pitch"
        case 5: return g.model == UInt32(ASMR_CLIP_MODEL_WATER) ? "flow" : "hiss"
        default: return g.model == UInt32(ASMR_CLIP_MODEL_WATER) ? "turbulence" : "whistlers"
        }
    }

    /// The clip groups as controls: a level lever per group (a macro) whose aspects are its
    /// children, so the panel draws them like the forest's and opens them the same way.
    var clipControls: [Control] {
        var out: [Control] = []
        for g in clipGroups where g.index < clipValues.count {
            let base = Self.clipControlBase + UInt32(g.index) * 8
            let group = Self.clipGroupNumber(g)
            for p in 0..<7 where g.has(p) {
                let r = g.ranges[p]
                let v = clipValues[g.index][p]
                let macro = p == 0
                var flags: UInt32 = macro ? UInt32(ASMR_CONTROL_FLAG_MACRO) : 0
                if !macro, abs(v - r.def) > 1e-4 { flags |= UInt32(ASMR_CONTROL_FLAG_OVERRIDDEN) }
                out.append(Control(id: base + UInt32(p), group: group, parent: macro ? nil : base, flags: flags,
                                   min: r.min, max: r.max, value: v, defaultValue: r.def,
                                   key: "clip.\(g.id).\(clipLeaf(g, p))", unit: macro ? "dB" : p == 6 && g.model == UInt32(ASMR_CLIP_MODEL_PLASMA) ? "/min" : "x"))
            }
        }
        return out
    }

    private func setClipGroup(_ id: UInt32, _ value: Float) {
        let k = Int(id - Self.clipControlBase)
        let (gi, p) = (k / 8, k % 8)
        guard gi < clipGroups.count, p < 7, clipGroups[gi].has(p) else { return }
        let r = clipGroups[gi].ranges[p]
        // NaN (double-click) goes back to "as authored".
        let v = value.isNaN ? r.def : min(max(value, r.min), r.max)
        clipValues[gi][p] = v
        UserDefaults.standard.set(v, forKey: clipDefaultsKey(clipGroups[gi], p))
        asmr_set_clip_group_param(core, UInt32(gi), UInt32(p), v)
    }

    /// Builds the chosen scene off the main thread (clip files are decoded and resampled), then
    /// swaps cores; playback, volume, head pose and screen carry over. Remembered across launches.
    func switchScene(to entry: SceneEntry) {
        guard entry != scene.entry || sceneError != nil, loadingScene != entry else { return }
        SceneEntry.saved = entry
        guard let info = SceneInfo.load(entry) else {
            sceneError = tr("No scene file")
            NSLog("ASMR X3000: scene \(entry.id) not found in \(Scenes.directory?.path ?? "-")")
            return
        }
        loadingScene = entry
        sceneError = nil
        let rate = coreRate
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let made = Self.makeCore(Float(rate), json: info.json)
            DispatchQueue.main.async {
                guard let self else {
                    if let made { asmr_engine_free(made) }
                    return
                }
                // A newer choice supersedes this one.
                guard self.loadingScene == entry else {
                    if let made { asmr_engine_free(made) }
                    return
                }
                self.loadingScene = nil
                guard let made else {
                    self.sceneError = tr("The scene failed to load")
                    NSLog("ASMR X3000: scene \(entry.id) did not build (missing sound files?)")
                    return
                }
                if rate != self.coreRate {
                    // The output device changed meanwhile: build again at the new rate.
                    asmr_engine_free(made)
                    self.switchScene(to: entry)
                    return
                }
                self.install(made, scene: info)
            }
        }
    }

    /// Swaps in a core built for `info`. The render callbacks hold the core pointer, so the
    /// graph is stopped and rebuilt around the swap.
    private func install(_ new: OpaquePointer, scene info: SceneInfo) {
        let volumeNow = asmr_get_param(core, UInt32(ASMR_PARAM_VOLUME))
        engine.stop()
        if let old = node {
            engine.detach(old)
            node = nil
        }
        // Flight is the forest's generated world; a clip scene always starts at home.
        if info.isClip || scene.isClip { place = .home }
        coreLock.lock()
        let old = core
        core = new
        scene = info
        controlNames.removeAll()
        optionNames.removeAll()
        directionNames.removeAll()
        directionLabels.removeAll()
        applyParams()
        set(ASMR_PARAM_VOLUME, volumeNow)
        coreLock.unlock()
        asmr_engine_free(old)
        // Paused (or fading out) stays paused; the device is released until the next play.
        if isPlaying { play() }
    }

    func setScreen(width: Float, height: Float, distance: Float, elevation: Float) {
        screen = (width, height, distance)
        screenElevation = elevation
        set(ASMR_PARAM_SCREEN_WIDTH, width)
        set(ASMR_PARAM_SCREEN_HEIGHT, height)
        set(ASMR_PARAM_SCREEN_DISTANCE, distance)
        set(ASMR_PARAM_SCREEN_ELEVATION, elevation)
    }

    /// The screen centre's angle from the eyes, degrees, − = below (ASMR_PARAM_SCREEN_ELEVATION).
    private(set) var screenElevation: Float?

    private let engine = AVAudioEngine()
    private var node: AVAudioSourceNode?
    private var core: OpaquePointer
    private var coreRate: Double = 48_000
    private var headPose: (Float, Float, Float) = (0, 0, 0)
    private var stopWork: DispatchWorkItem?
    private var rawSources = [AsmrSourceInfo](repeating: AsmrSourceInfo(), count: 128)
    private var rawObjects = [AsmrObjectInfo](repeating: AsmrObjectInfo(), count: 256)
    private var rawControls = [AsmrControlInfo](repeating: AsmrControlInfo(), count: 96)
    private var controlNames: [UInt32: (String, String)] = [:]

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
        Scenes.prepareEnvironment()
        // `--scene path` (development, snapshots) overrides the remembered choice.
        var info = Self.sceneArgument.flatMap(SceneInfo.load(path:)) ?? SceneInfo.load(.saved) ?? .forest
        var made = Self.makeCore(48_000, json: info.json)
        if made == nil {
            NSLog("ASMR X3000: scene \(info.entry.id) did not build, using the forest")
            info = .forest
            made = Self.makeCore(48_000, json: nil)
        }
        scene = info
        core = made!
        applyParams()
        if renderer != .core { prepareSpatialMixer() }
        NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
        ) { [weak self] _ in
            self?.handleConfigurationChange()
        }
    }

    /// `--scene path`: a JSON scene file for development and snapshots.
    private static let sceneArgument: String? = {
        let args = CommandLine.arguments
        guard let i = args.firstIndex(of: "--scene"), i + 1 < args.count else { return nil }
        return args[i + 1]
    }()

    /// The core for a scene: nil when the JSON does not build; without JSON the built-in forest.
    private static func makeCore(_ rate: Float, json: String?) -> OpaquePointer? {
        guard let json else { return asmr_engine_new(rate) }
        return asmr_engine_new_with_scene(rate, json)
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
            outputLevel: telemetry(ASMR_TELEMETRY_OUTPUT_LEVEL),
            lightning: telemetry(ASMR_TELEMETRY_LIGHTNING),
            wetness: telemetry(ASMR_TELEMETRY_WETNESS),
            flight: telemetry(ASMR_TELEMETRY_FLIGHT),
            altitude: telemetry(ASMR_TELEMETRY_FLIGHT_ALTITUDE),
            odometer: telemetry(ASMR_TELEMETRY_FLIGHT_ODOMETER),
            speed: telemetry(ASMR_TELEMETRY_FLIGHT_SPEED),
            heading: telemetry(ASMR_TELEMETRY_FLIGHT_HEADING),
            worldX: telemetry(ASMR_TELEMETRY_FLIGHT_X),
            worldY: telemetry(ASMR_TELEMETRY_FLIGHT_Y),
            loudness: telemetry(ASMR_TELEMETRY_LOUDNESS_SHORT),
            mixGain: telemetry(ASMR_TELEMETRY_MIX_GAIN_DB),
            soloists: telemetry(ASMR_TELEMETRY_SOLOISTS),
            masking: telemetry(ASMR_TELEMETRY_MASKING)
        )
    }

    /// Full sound-director telemetry: per-group loudness, masking and ducks, the soloists.
    func mixInfo() -> AsmrMixInfo {
        var info = AsmrMixInfo()
        asmr_get_mix(core, &info)
        return info
    }

    /// Scene objects (id, kind, position, flags). Ids are stable for the engine's lifetime.
    func readObjects(_ out: inout [AsmrObjectInfo]) {
        let n = rawObjects.withUnsafeMutableBufferPointer {
            asmr_get_objects(core, $0.baseAddress, UInt32($0.count))
        }
        out.removeAll(keepingCapacity: true)
        out.append(contentsOf: rawObjects.prefix(Int(n)))
    }

    /// Flight mode: where the listener is in the landscape and what it is (nil before flight
    /// mode was first built). See Stage/FlightTerrain.swift.
    func readFlightFrame() -> AsmrFlightFrame? {
        var f = AsmrFlightFrame()
        return asmr_get_flight_frame(core, &f) != 0 ? f : nil
    }

    /// Actual rain (or snow) rate, mm/h, for the precipitation drawn over the scene.
    var rainRate: Float { telemetry(ASMR_TELEMETRY_RAIN_RATE) }

    /// What each scene bird is doing (species, state, flight style, wingbeat): Stage/Birds.swift.
    func readBirds(_ out: inout [AsmrBirdInfo]) -> Int {
        out.withUnsafeMutableBufferPointer { Int(asmr_get_birds(core, $0.baseAddress, UInt32($0.count))) }
    }

    /// Shape of a streamed object (river bed, lake shore, ridge line), listener-relative.
    func readObjectPath(_ id: UInt32, into out: inout [SIMD3<Float>]) {
        var raw = [Float](repeating: 0, count: 3 * 32)
        let n = raw.withUnsafeMutableBufferPointer { asmr_get_object_path(core, id, $0.baseAddress, 32) }
        out.removeAll(keepingCapacity: true)
        for i in 0..<Int(n) { out.append(SIMD3(raw[3 * i], raw[3 * i + 1], raw[3 * i + 2])) }
    }

    /// The control tree: macros and their children. Values change when a macro moves, so this is
    /// read every frame.
    func readControls(_ out: inout [Control]) {
        let n = rawControls.withUnsafeMutableBufferPointer {
            asmr_get_controls(core, $0.baseAddress, UInt32($0.count))
        }
        out.removeAll(keepingCapacity: true)
        let hasDirection = asmr_direction_episode_count(core) > 0
        for var raw in rawControls.prefix(Int(n)) {
            if !hasDirection, Int(raw.group) == ASMR_GROUP_DIRECTION { continue }   // a clip scene has no course of its own
            let names: (String, String)
            if let cached = controlNames[raw.id] {
                names = cached
            } else {
                let key = withUnsafeBytes(of: &raw.key) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
                let unit = withUnsafeBytes(of: &raw.unit) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
                names = (key, unit)
                controlNames[raw.id] = names
            }
            var c = Control(id: raw.id, group: raw.group, parent: raw.parent == UInt32.max ? nil : raw.parent,
                            flags: raw.flags, min: raw.min, max: raw.max, value: raw.value,
                            defaultValue: raw.default_value, key: names.0, unit: names.1)
            if c.isDiscrete { c.options = options(for: c) }
            out.append(c)
        }
        if scene.isClip {
            out.append(contentsOf: clipControls)
            out.append(contentsOf: sceneControls)
        }
    }

    private var optionNames: [UInt32: [String]] = [:]

    private func options(for c: Control) -> [String] {
        if let cached = optionNames[c.id] { return cached }
        var names: [String] = []
        var buf = [CChar](repeating: 0, count: 64)
        for i in 0...max(Int(c.max), 0) {
            let n = buf.withUnsafeMutableBufferPointer { asmr_get_control_option(core, c.id, UInt32(i), $0.baseAddress, 64) }
            let name = n > 0 ? String(cString: buf) : ""
            // "steady_rain" is also a rain surface: the cue's spells are told apart by a prefix.
            names.append(c.key == "direction.cue" && !name.isEmpty ? "cue." + name : name)
        }
        optionNames[c.id] = names
        return names
    }

    /// Where the world's course stands (asmr_get_direction); `available` = 0 in a scene without one.
    func directionInfo() -> AsmrDirectionInfo {
        var info = AsmrDirectionInfo()
        asmr_get_direction(core, &info)
        return info
    }

    private var directionNames: [UInt32: String] = [:]
    private var directionLabels: [UInt32: String] = [:]

    /// Machine name of a spell ("thunderstorm"); cached.
    func directionEpisodeName(_ episode: UInt32) -> String {
        if let cached = directionNames[episode] { return cached }
        var buf = [CChar](repeating: 0, count: 64)
        let n = buf.withUnsafeMutableBufferPointer { asmr_direction_episode_name(core, episode, $0.baseAddress, 64) }
        let name = n > 0 ? String(cString: buf) : ""
        directionNames[episode] = name
        return name
    }

    /// Label of one stretch of a spell ("gust front"); cached.
    func directionSegmentLabel(episode: UInt32, segment: UInt32) -> String {
        let key = episode << 16 | segment
        if let cached = directionLabels[key] { return cached }
        var buf = [CChar](repeating: 0, count: 64)
        let n = buf.withUnsafeMutableBufferPointer { asmr_direction_segment_label(core, episode, segment, $0.baseAddress, 64) }
        let label = n > 0 ? String(cString: buf) : ""
        directionLabels[key] = label
        return label
    }

    func setControl(_ id: UInt32, _ value: Float) {
        if id >= Self.clipControlBase {
            setClipGroup(id, value)
            return
        }
        if id >= Self.sceneControlBase, let m = SceneMacro(rawValue: Int(id - Self.sceneControlBase)) {
            // NaN (double-click) goes back to "as authored".
            sceneMacros[m.rawValue] = value.isNaN ? 1 : min(max(value, 0), m.max)
            set(m.param, sceneMacros[m.rawValue])
            return
        }
        asmr_set_control(core, id, value)
    }

    /// Hands a child back to its macro.
    func releaseControl(_ id: UInt32) { setControl(id, .nan) }

    /// Live peak waveform of a track (ASMR_SCOPE_*), oldest first, one point per 64 samples.
    func readScope(_ track: Int, into out: inout [Float]) {
        out.withUnsafeMutableBufferPointer { _ = asmr_get_scope(core, UInt32(track), $0.baseAddress, UInt32($0.count)) }
    }

    func setObjectPosition(id: UInt32, _ p: SIMD3<Float>) {
        asmr_set_object_position(core, id, p.x, p.y, p.z)
    }

    // MARK: Prompt control (docs/prompt-control.md)

    /// System prompt, current-state context and tool schema (asmr_get_prompt_json).
    func promptJSON() -> Data { Data(readString { asmr_get_prompt_json(core, $0, $1) }.utf8) }

    /// Controls, environment and objects (asmr_get_state_json).
    func stateJSON() -> Data { Data(readString { asmr_get_state_json(core, $0, $1) }.utf8) }

    /// Applies a patch; returns the core's result JSON (ok, applied, errors, undo).
    @discardableResult
    func applyPatch(_ patch: Data) -> [String: Any] {
        let text = String(decoding: patch, as: UTF8.self)
        guard let resultPointer = asmr_apply_patch_json_alloc(core, text) else { return [:] }
        defer { asmr_string_free(resultPointer) }
        let result = String(cString: resultPointer)
        let parsed = (try? JSONSerialization.jsonObject(with: Data(result.utf8))) as? [String: Any] ?? [:]
        // A patch may switch flight on or off (the core already set the parameter): keep the
        // published state, the transport button and rebuilds in step.
        if let f = (parsed["switched"] as? [String: Any])?["flight.on"] as? String {
            let to: Place = f == "on" ? .flying : f == "stand" ? .standing : .home
            let sync = { [weak self] in if let self, self.place != to { self.place = to } }
            if Thread.isMainThread { sync() } else { DispatchQueue.main.async(execute: sync) }
        }
        return parsed
    }

    /// Calls a "length-returning" ABI function twice: once to size, once to fill.
    private func readString(_ call: (UnsafeMutablePointer<CChar>?, UInt32) -> UInt32) -> String {
        var size = Int(call(nil, 0)) + 1
        while true {
            var buf = [CChar](repeating: 0, count: size)
            let n = Int(buf.withUnsafeMutableBufferPointer { call($0.baseAddress, UInt32(size)) })
            if n < size { return String(cString: buf) }
            size = n + 1
        }
    }

    /// Restore actual values and override flags, independent of how they were edited.
    /// Macros precede children so a reset resumes following the restored macro.
    func restoreState(_ state: Data) {
        guard let root = (try? JSONSerialization.jsonObject(with: state)) as? [String: Any],
              let controls = root["controls"] as? [[String: Any]] else { return }
        let ordered = controls.filter { $0["macro"] as? Bool == true }
            + controls.filter { $0["macro"] as? Bool != true }
        let changes: [[String: Any]] = ordered.compactMap { c in
            guard let key = c["key"] as? String, let value = c["target"] ?? c["value"] else { return nil }
            return ["key": key, "value": value]
        }
        for object in root["objects"] as? [[String: Any]] ?? [] {
            guard object["movable"] as? Bool == true, let id = object["id"] as? NSNumber,
                  let pos = object["pos"] as? [NSNumber], pos.count == 3 else { continue }
            // Restoring a scene position must not apply the prompt's on-stage clamps.
            asmr_set_object_position(core, id.uint32Value, pos[0].floatValue, pos[1].floatValue, pos[2].floatValue)
        }
        guard let patch = try? JSONSerialization.data(withJSONObject: ["glide_s": 0, "changes": changes]) else { return }
        applyPatch(patch)
        // Clear override flags without changing stored values of children that have no
        // macro curve (e.g. a released gain control). A JSON preset reset also resets value.
        let followers = controls.filter {
            $0["target"] as? String == "reset"
                || ($0["target"] == nil && $0["overridden"] as? Bool == false)
        }
        var tree: [Control] = []
        readControls(&tree)
        for follower in followers {
            guard let control = tree.first(where: { $0.key == follower["key"] as? String }) else { continue }
            asmr_set_control(core, control.id, .nan)
        }
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
        set(ASMR_PARAM_FLIGHT, Float(place.rawValue))
        set(ASMR_PARAM_PLAYING, isPlaying ? 1 : 0)
        setHeadPose(yaw: headPose.0, pitch: headPose.1, roll: headPose.2)
        if let s = lastHead {
            asmr_set_head_pose(core, s.q.real, s.q.imag.x, s.q.imag.y, s.q.imag.z, s.w.x, s.w.y, s.w.z, s.age)
        }
        if outputLatency > 0 { set(ASMR_PARAM_OUTPUT_LATENCY, Float(outputLatency)) }
        if let screen {
            set(ASMR_PARAM_SCREEN_WIDTH, screen.width)
            set(ASMR_PARAM_SCREEN_HEIGHT, screen.height)
            set(ASMR_PARAM_SCREEN_DISTANCE, screen.distance)
        }
        if let screenElevation { set(ASMR_PARAM_SCREEN_ELEVATION, screenElevation) }
        for m in SceneMacro.allCases { set(m.param, sceneMacros[m.rawValue]) }
        refreshClipGroups()
        // Listening-check overrides (docs/spatial-engine.md 6.5).
        let d = UserDefaults.standard
        if d.object(forKey: "headPrediction") != nil { set(ASMR_PARAM_HEAD_PREDICTION, d.float(forKey: "headPrediction")) }
        if d.object(forKey: "headParallax") != nil { set(ASMR_PARAM_HEAD_PARALLAX, d.float(forKey: "headParallax")) }
    }

    /// Must only run while the AVAudioEngine is stopped.
    private func rebuild(sampleRate rate: Double) {
        if let old = node {
            engine.detach(old)
            node = nil
        }
        if rate != coreRate {
            coreLock.lock()
            let state = stateJSON()
            let old = core
            core = Self.makeCore(Float(rate), json: scene.json) ?? Self.makeCore(Float(rate), json: nil)!
            coreRate = rate
            applyParams()
            restoreState(state)
            asmr_engine_free(old)
            coreLock.unlock()
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
