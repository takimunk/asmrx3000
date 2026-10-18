import AppKit
import CAsmr
import QuartzCore
import simd

/// Something the user can point at: the river (all water segments), a tree (a canopy wind
/// source) or a bird.
struct Thing {
    enum Kind { case river, tree, bird }
    enum Key: Hashable { case river, object(UInt32) }

    var key: Key
    var kind: Kind
    var ids: [UInt32]
    /// Canopy centre, bird body, or the mean river point.
    var anchor: SIMD3<Float>
    var movable: Bool
    var boxMin: SIMD3<Float>
    var boxMax: SIMD3<Float>
}

/// What the scene readout shows about the selection.
struct SelectionInfo: Equatable {
    var kind: Thing.Kind
    var position: SIMD3<Float>
    var movable: Bool
}

final class SceneReadout: ObservableObject {
    @Published var selection: SelectionInfo?
}

/// The control tree as the panel shows it; published only when a value changes.
final class ControlsModel: ObservableObject {
    @Published var controls: [Control] = []
    /// The macro whose branch is open (one at a time).
    @Published var expanded: UInt32?

    var macros: [Control] { controls.filter { $0.parent == nil } }
    func children(of id: UInt32) -> [Control] { controls.filter { $0.parent == id } }
}

/// Small orientation labels over the scene; published only when their text or place changes.
final class SceneLabels: ObservableObject {
    /// Time of day, e.g. "21:30".
    @Published var time = ""
    /// Where the head is turned, e.g. "35° left, 15° up", or nil while head tracking is off.
    @Published var gaze: String?
}

final class Meters: ObservableObject {
    /// Tracks that are audible right now; changes only when one starts or stops sounding.
    @Published var audible: Set<Track> = []
    /// True while the popover is on screen (icons only animate then).
    @Published var live = false
}

/// Order matches the rows of the track scopes and ASMR_SCOPE_*.
enum Track: Int, CaseIterable {
    case water, wind, birds, flight, volume, rain, insects, fire, thunder
}

/// Per-frame state shared by the scene and spectrogram renderers. Telemetry is sampled once per
/// display frame, only while a view is on screen.
final class Stage {
    /// Drawn height of a tree for picking, m (the ABI reports trunk bases only).
    static let treeHeight: Float = 8
    static let bands = 160
    static let columns = 720
    static let columnsPerSecond = 60.0

    let audio: AudioEngine
    let head: HeadTracker
    let readout = SceneReadout()
    let meters = Meters()
    let labels = SceneLabels()
    /// Size of the scene view in points (set by the view as it draws).
    var sceneSize: SIMD2<Float> = .zero
    let controls = ControlsModel()
    private var controlBuffer: [Control] = []

    var camera = OrbitCamera()
    /// Views drawn in the scene view's frame (see StageView).
    let followers = NSHashTable<StageView>.weakObjects()
    private(set) var selection: Thing.Key?
    var hover: Thing.Key?

    private(set) var spectrum = [Float](repeating: 0, count: Stage.bands)
    private(set) var wind = [Float](repeating: 0, count: 64)
    private(set) var sources: [SourceInfo] = []
    private(set) var objects: [AsmrObjectInfo] = []
    private(set) var things: [Thing] = []
    private(set) var telemetry = Telemetry()
    /// Flight mode: telemetry and the shapes of the streamed water (see FlightStage.swift).
    var flight = FlightView()
    /// Camera target height outside flight mode; in flight the view follows the herm up.
    /// The camera the scene is drawn and picked with: the user's orbit camera, and in flight
    /// tilted down onto the canopy and centred on the herm, so the ground streaming below and
    /// the herm stay in view at any altitude.
    var viewCamera: OrbitCamera {
        var c = camera
        let m = min(max(flight.mix, 0), 1)
        guard m > 0 else { return c }
        let lift = min(0.5 + 0.012 * flight.altitude, 0.8)
        c.elevation += (max(c.elevation, lift) - c.elevation) * m
        c.target.x *= 1 - m
        c.target.z -= 0.35 * flight.drop * m
        return c
    }

    /// Spectrogram history: `columns` columns of `bands` floats, column-major.
    private(set) var history = [Float](repeating: 0, count: Stage.columns * Stage.bands)
    /// Total columns written; the newest is at `(written - 1) % columns`.
    private(set) var written = 0
    var frozen = false

    /// Live waveform per track for the sliders: `scopeColumns` peaks each, 0..1, oldest first.
    static let scopeColumns = 512
    private(set) var scopes = [Float](repeating: 0, count: Track.allCases.count * Stage.scopeColumns)
    private var scopeRaw = [Float](repeating: 0, count: Stage.scopeColumns * 2)
    private var scopeMax = [Float](repeating: 0.05, count: Track.allCases.count)
    /// Raw peak of each track over the last ~85 ms: what "audible" means for the icons.
    private var recentPeak = [Float](repeating: 0, count: Track.allCases.count)

    /// Scene time for animation, seconds.
    private(set) var time: Double = 0
    private var lastTick: CFTimeInterval = 0
    private var columnClock: Double = 0
    private var lastMeter: CFTimeInterval = 0
    /// Until when the scene runs at the higher frame rate.
    private(set) var activeUntil: CFTimeInterval = 0

    /// Called on user input: run smoothly for a moment, then settle back to the rest rate.
    func poke() {
        activeUntil = CACurrentMediaTime() + 1.5
    }

    /// Display frames sampled so far (for --bench).
    private(set) var frames = 0

    init(audio: AudioEngine, head: HeadTracker) {
        self.audio = audio
        self.head = head
    }

    // MARK: Frame

    /// Samples telemetry. Safe to call from several views per frame; only the first does work.
    func tick(now: CFTimeInterval = CACurrentMediaTime()) {
        let dt = lastTick == 0 ? 1 / 60 : min(now - lastTick, 0.1)
        guard dt > 1 / 240 else { return }
        lastTick = now
        frames += 1
        step(dt: dt)
        if now - lastMeter > 1 / 10 {
            lastMeter = now
            publishMeters()
            updateLabels()
        }
    }

    /// Advances by a fixed step (snapshot mode, no display clock).
    func step(dt: Double) {
        time += dt
        telemetry = audio.readTelemetry(spectrum: &spectrum, windField: &wind, sources: &sources)
        audio.readObjects(&objects)
        readFlight()
        audio.readControls(&controlBuffer)
        if controlBuffer != controls.controls { controls.controls = controlBuffer }
        rebuildThings()

        columnClock += dt * Stage.columnsPerSecond
        var due = min(Int(columnClock), 8)
        columnClock -= Double(Int(columnClock))
        if frozen { due = 0 }
        while due > 0 {
            let col = written % Stage.columns
            history.withUnsafeMutableBufferPointer { h in
                for b in 0..<Stage.bands { h[col * Stage.bands + b] = spectrum[b] }
            }
            written += 1
            due -= 1
        }
        readScopes()
        refreshReadout()
    }

    /// Re-reads the control tree right away (after the user moved a slider).
    func refreshControls() {
        audio.readControls(&controlBuffer)
        if controlBuffer != controls.controls { controls.controls = controlBuffer }
    }

    /// Head pose as the scene shows it (engine convention: yaw + = left, pitch + = up, radians)
    /// and whether it is live. `gazeOverride` stands in for AirPods in snapshots.
    var gazeOverride: (yaw: Float, pitch: Float)?
    var headPose: (yaw: Float, pitch: Float, live: Bool) {
        if let g = gazeOverride { return (g.yaw, g.pitch, true) }
        let live = head.enabled && head.active
        return live ? (Float(head.yaw), Float(head.pitch), true) : (0, 0, false)
    }

    /// Where the listener looks: head (origin) + forward × `gazeDistance` metres. Same rotation
    /// as Mat3::from_head in the engine, so the dot points where the sound is centred.
    static let gazeDistance: Float = 3.5
    var gazePoint: SIMD3<Float> {
        let p = headPose
        return Stage.gazeDistance * SIMD3(cos(p.pitch) * cos(p.yaw), cos(p.pitch) * sin(p.yaw), sin(p.pitch))
    }

    /// The scene's time of day, hours.
    var hour: Float { controls.controls.first { $0.key == "env.time_of_day" }?.value ?? 12 }

    func setLive(_ live: Bool) {
        if meters.live != live { meters.live = live }
    }

    /// Two engine blocks per column (~2.7 ms), so a slider shows the last ~1.4 s. Each track is
    /// scaled by a slowly decaying peak so quiet sounds still read.
    private func readScopes() {
        let n = Stage.scopeColumns
        for t in Track.allCases {
            audio.readScope(t.rawValue, into: &scopeRaw)
            var peak: Float = 0
            for c in 0..<n { peak = max(peak, max(scopeRaw[2 * c], scopeRaw[2 * c + 1])) }
            recentPeak[t.rawValue] = scopeRaw.suffix(64).max() ?? 0
            let m = max(scopeMax[t.rawValue] * 0.995, peak, 0.02)
            scopeMax[t.rawValue] = m
            let base = t.rawValue * n
            for c in 0..<n {
                scopes[base + c] = (max(scopeRaw[2 * c], scopeRaw[2 * c + 1]) / m).squareRoot()
            }
        }
    }

    func updateLabels() {
        guard sceneSize.x > 0 else { return }

        let h = hour
        let minutes = Int((h * 60).rounded()) % (24 * 60)
        let time = String(format: "%02d:%02d", minutes / 60, minutes % 60)
        if labels.time != time { labels.time = time }

        let p = headPose
        var gaze: String?
        if p.live {
            // Rounded to 5° so the text holds still while the head drifts.
            let yaw = Int((p.yaw * 180 / .pi / 5).rounded()) * 5
            let pitch = Int((p.pitch * 180 / .pi / 5).rounded()) * 5
            let side = yaw == 0 ? "straight ahead" : "\(abs(yaw))° \(yaw > 0 ? "left" : "right")"
            gaze = pitch == 0 ? side : "\(side), \(abs(pitch))° \(pitch > 0 ? "up" : "down")"
        }
        if labels.gaze != gaze { labels.gaze = gaze }
    }

    private func publishMeters() {
        let audible = Set(Track.allCases.filter { recentPeak[$0.rawValue] > 0.003 })
        if meters.audible != audible { meters.audible = audible }
    }

    // MARK: Things

    private func rebuildThings() {
        things.removeAll(keepingCapacity: true)
        var riverIDs: [UInt32] = []
        var riverMin = SIMD3<Float>(repeating: .infinity), riverMax = SIMD3<Float>(repeating: -.infinity)
        var riverSum = SIMD3<Float>(repeating: 0)
        for o in objects {
            let p = SIMD3(o.x, o.y, o.z)
            let movable = o.flags & UInt32(ASMR_OBJECT_FLAG_MOVABLE) != 0 && o.flags & UInt32(ASMR_OBJECT_FLAG_MOVING) == 0
            if o.flags & UInt32(ASMR_OBJECT_FLAG_ABSENT) != 0 { continue }
            // In flight the landscape streams past: only what is on the disc can be pointed at.
            if flight.active && (o.kind == UInt32(ASMR_OBJECT_RIVER) || simd_length(SIMD2(p.x, p.y)) > Terrain.radius) { continue }
            switch o.kind {
            case UInt32(ASMR_OBJECT_RIVER):
                // The engine reports the river's centroid; its extent comes from the water emitters.
                riverIDs.append(o.id)
                for w in riverPoints() {
                    let q = SIMD3(w.x, w.y, Terrain.waterLevel(w.z))
                    riverMin = simd_min(riverMin, q)
                    riverMax = simd_max(riverMax, q)
                }
                riverSum = p
            case UInt32(ASMR_OBJECT_TREE):
                let g = flight.active ? FlightGround.height(p.x, p.y, odometer: flight.odometer) - flight.drop : Terrain.height(p.x, p.y)
                things.append(Thing(key: .object(o.id), kind: .tree, ids: [o.id], anchor: SIMD3(p.x, p.y, g), movable: movable,
                                    boxMin: SIMD3(p.x - 2.0, p.y - 2.0, g), boxMax: SIMD3(p.x + 2.0, p.y + 2.0, g + Self.treeHeight)))
            case UInt32(ASMR_OBJECT_BIRD):
                let r: Float = 0.45
                things.append(Thing(key: .object(o.id), kind: .bird, ids: [o.id], anchor: p, movable: movable,
                                    boxMin: p - r, boxMax: p + r))
            default:
                break // generic sources (rain patches, crickets, fire) are not drawn yet
            }
        }
        if !riverIDs.isEmpty {
            let mean = riverSum
            things.append(Thing(key: .river, kind: .river, ids: riverIDs, anchor: mean, movable: true,
                                boxMin: riverMin - SIMD3(1.3, 0.6, 0.1), boxMax: riverMax + SIMD3(1.3, 0.6, 0.1)))
        }
    }

    func thing(_ key: Thing.Key?) -> Thing? {
        guard let key else { return nil }
        return things.first { $0.key == key }
    }

    /// Ids flagged in the shader masks: (selected bits, hovered bits, river selected, river hovered).
    var masks: SIMD4<UInt32> {
        func bits(_ key: Thing.Key?) -> UInt32 {
            guard let t = thing(key), t.kind != .river else { return 0 }
            return t.ids.reduce(0) { $1 < 32 ? $0 | (1 << $1) : $0 }
        }
        return SIMD4(bits(selection), hover == selection ? 0 : bits(hover),
                     selection == .river ? 1 : 0, hover == .river && selection != .river ? 1 : 0)
    }

    // MARK: Interaction

    /// The thing under a point in a view of `size` points, if any.
    func pick(at point: SIMD2<Float>, size: SIMD2<Float>) -> Thing.Key? {
        let proj = Projector(camera: viewCamera, size: size)
        let focal = size.y * 0.5 / tan(camera.fovY / 2)
        var best: (Thing.Key, Float)?
        func consider(_ key: Thing.Key, _ world: SIMD3<Float>, _ radius: Float) {
            guard let s = proj.project(world) else { return }
            let r = max(radius * focal / s.w, 7)
            let d = simd_distance(s.point, point) / r
            if d < 1, best == nil || d < best!.1 { best = (key, d) }
        }
        for t in things {
            switch t.kind {
            case .tree:
                consider(t.key, t.anchor, 1.5)
                let g = Terrain.height(t.anchor.x, t.anchor.y)
                consider(t.key, SIMD3(t.anchor.x, t.anchor.y, (g + t.anchor.z) / 2), 0.7)
            case .bird:
                consider(t.key, t.anchor, 0.5)
            case .river:
                break
            }
        }
        if best == nil, let river = thing(.river) {
            // Walk the river line; it is selectable anywhere along its length.
            let pts = riverPoints()
            var y: Float = -Terrain.radius
            while y <= Terrain.radius {
                let xz = Self.riverAt(y, pts)
                consider(river.key, SIMD3(xz.x, y, Terrain.waterLevel(xz.y)), 1.1)
                y += 0.5
            }
        }
        return best?.0
    }

    func select(_ key: Thing.Key?) {
        selection = key
        refreshReadout()
    }

    /// Selects the next thing in a stable order.
    func cycleSelection(backwards: Bool = false) {
        let keys = things.map(\.key)
        guard !keys.isEmpty else { return }
        let i = selection.flatMap { keys.firstIndex(of: $0) } ?? (backwards ? 0 : -1)
        select(keys[(i + (backwards ? -1 : 1) + keys.count) % keys.count])
    }

    /// Moves the selection by a camera-relative step: x = to the right on screen,
    /// y = away from the camera, z = up.
    func nudge(_ d: SIMD3<Float>) {
        guard let t = thing(selection), t.movable else { return }
        let axes = camera.screenAxes
        let ground = axes.right * d.x + axes.forward * d.y
        let delta = SIMD3(ground.x, ground.y, d.z)
        let limit = Terrain.radius - 1.2
        for id in t.ids {
            guard let o = objects.first(where: { $0.id == id }) else { continue }
            var p = SIMD3(o.x, o.y, o.z) + delta
            let r = simd_length(SIMD2(p.x, p.y))
            if r > limit { p.x *= limit / r; p.y *= limit / r }
            let floor = Terrain.height(p.x, p.y) + (t.kind == .tree ? 1.2 : t.kind == .river ? 0.4 : 0.3)
            p.z = min(max(p.z, floor), 12)
            audio.setObjectPosition(id: id, p)
            if let i = objects.firstIndex(where: { $0.id == id }) {
                objects[i].x = p.x; objects[i].y = p.y; objects[i].z = p.z
            }
        }
        rebuildThings()
        refreshReadout()
    }

    private func refreshReadout() {
        let info = thing(selection).map { SelectionInfo(kind: $0.kind, position: $0.anchor, movable: $0.movable) }
        if readout.selection != info { readout.selection = info }
    }

    // MARK: River

    /// River sources as (x, y, z, level), sorted by y. Level comes from the source telemetry.
    func riverPoints() -> [SIMD4<Float>] {
        if flight.active { return flight.river }
        var pts: [SIMD4<Float>] = []
        // Only the river's own segments: other water sources (drips, a waterfall) sit elsewhere.
        let centre = objects.first { $0.kind == UInt32(ASMR_OBJECT_RIVER) }.map { SIMD2($0.x, $0.y) }
        for s in sources where s.kind == .water {
            if let c = centre, abs(s.world.x - c.x) > 3 { continue }
            pts.append(SIMD4(s.world.x, s.world.y, s.world.z, s.level))
        }
        pts.sort { $0.y < $1.y }
        // The shader takes at most 8 points: keep the ends and spread the rest.
        guard pts.count > 8 else { return pts }
        return (0..<8).map { pts[$0 * (pts.count - 1) / 7] }
    }

    /// Same interpolation as `riverAt()` in the shader: (x, z) of the river at lateral `y`.
    static func riverAt(_ y: Float, _ pts: [SIMD4<Float>]) -> SIMD2<Float> {
        guard let first = pts.first, let last = pts.last else { return SIMD2(4, -1.2) }
        if y <= first.y { return SIMD2(first.x, first.z) }
        for i in 0..<(pts.count - 1) where y <= pts[i + 1].y {
            let a = pts[i], b = pts[i + 1]
            var t = (y - a.y) / max(b.y - a.y, 1e-3)
            t = t * t * (3 - 2 * t)
            return simd_mix(SIMD2(a.x, a.z), SIMD2(b.x, b.z), SIMD2(repeating: t))
        }
        return SIMD2(last.x, last.z)
    }

    // MARK: Spectrogram

    static let fLo: Float = 40
    static let fHi: Float = 16_000

    /// Visible band range as fractions of the full log range.
    var zoom = SIMD2<Float>(0, 1)

    func frequency(atFraction f: Float) -> Float {
        Stage.fLo * pow(Stage.fHi / Stage.fLo, f)
    }

    func fraction(forFrequency hz: Float) -> Float {
        log(hz / Stage.fLo) / log(Stage.fHi / Stage.fLo)
    }

    /// History value at a fraction of the visible width (1 = newest) and band fraction.
    func historyValue(column back: Int, band fraction: Float) -> Float {
        guard written > 0 else { return 0 }
        let col = ((written - 1 - back) % Stage.columns + Stage.columns) % Stage.columns
        let b = min(max(Int(fraction * Float(Stage.bands - 1) + 0.5), 0), Stage.bands - 1)
        return history[col * Stage.bands + b]
    }
}
