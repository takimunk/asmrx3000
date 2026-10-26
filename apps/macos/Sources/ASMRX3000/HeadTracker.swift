import CoreMotion
import Foundation
import simd

/// Head orientation from AirPods (Pro / Max / 3rd gen+) and supported Beats via CoreMotion.
/// Only the pose is taken from Apple; all spatial rendering happens in asmr-core, which also
/// predicts the pose forward over the output latency (core/src/headpose.rs).
///
/// Pipeline: CMHeadphoneMotionManager (~25 Hz) on a private queue -> full attitude quaternion +
/// gyroscope rate, mapped into the engine frame (+x forward, +y left, +z up) -> recentred about
/// the vertical only (gravity stays absolute) -> `onSample` straight from the motion queue (the
/// engine side is lock-free), so a busy main thread never delays the pose.
///
/// CMHeadphoneMotionManager exists on macOS 14 and later only: on Ventura the tracker stays
/// off for good (`supported` is false) and the app plays without head tracking.
final class HeadTracker: NSObject, ObservableObject {
    static var supported: Bool {
        if #available(macOS 14, *) { return true }
        return false
    }

    @Published private(set) var connected = false
    @Published private(set) var active = false
    /// Remembered across launches ("headEnabled"): continuing onboarding without head tracking
    /// turns it off, and the app then does not ask for motion access at the next launch.
    @Published var enabled = HeadTracker.supported && (UserDefaults.standard.object(forKey: "headEnabled") as? Bool ?? true) {
        didSet {
            if !Self.supported, enabled { enabled = false; return }
            UserDefaults.standard.set(enabled, forKey: "headEnabled")
            if enabled { deferred = false }
            enabled ? start() : stop()
        }
    }
    /// The system refused motion access (Privacy & Security), or the request failed.
    @Published private(set) var denied = false
    /// Uptime of the last pose that reached the main thread (0 = none yet): onboarding watches it
    /// to tell a live link from a stale one.
    private(set) var lastSampleTime: TimeInterval = 0
    /// Until onboarding has explained why, the tracker does not start (starting is what makes
    /// the system ask for motion access). `requestAccess()` lifts it.
    private var deferred = false

    /// For the UI (radar), radians, engine convention: yaw + = left, pitch + = up,
    /// roll + = right ear down. Updated on the main thread at <= 30 Hz (not published: the
    /// stage polls them every frame).
    private(set) var yaw: Double = 0
    private(set) var pitch: Double = 0
    private(set) var roll: Double = 0

    struct Sample {
        /// Head -> world rotation in the engine frame.
        var q: simd_quatf
        /// Angular velocity in the head frame, rad/s (+z = turning left).
        var w: SIMD3<Float>
        /// Seconds since the sensor measured it.
        var age: Float
    }

    /// Called on the motion queue for every sample, and with identity when tracking stops.
    var onSample: ((Sample) -> Void)?

    /// How the headphones' sensor axes sit on the head. CoreMotion documents the axes only in a
    /// picture; `rightForwardUp` (x = towards the right ear, y = forward, z = up) is the
    /// reading under which CMAttitude's Euler angles mean what their names say (pitch + = look
    /// up, roll + = right ear down, yaw + = turn left). The other reading flips pitch and roll
    /// but not yaw. Stored in UserDefaults ("headAxes"), so the listening check can switch it.
    enum Axes: String { case rightForwardUp, leftBackUp }
    var axes: Axes = Axes(rawValue: UserDefaults.standard.string(forKey: "headAxes") ?? "") ?? .rightForwardUp {
        didSet { UserDefaults.standard.set(axes.rawValue, forKey: "headAxes") }
    }

    /// Slow yaw-drift correction: while the head is still and within +-20 deg of forward,
    /// "forward" creeps towards where the head points at 0.5 deg/s. AirPods have no
    /// magnetometer, so their yaw drifts by a few degrees per minute.
    var driftCorrection = UserDefaults.standard.object(forKey: "headDriftCorrection") as? Bool ?? true {
        didSet { UserDefaults.standard.set(driftCorrection, forKey: "headDriftCorrection") }
    }

    /// A MotionLink on macOS 14+, nil before.
    private var link: AnyObject?
    @available(macOS 14, *)
    private var manager: CMHeadphoneMotionManager? { (link as? MotionLink)?.manager }
    private let queue: OperationQueue = {
        let q = OperationQueue()
        q.name = "asmr.headtracker"
        q.maxConcurrentOperationCount = 1
        q.qualityOfService = .userInteractive
        return q
    }()
    // Motion-queue state.
    private var heading: Float = 0 // yaw of "forward" in CoreMotion's reference frame
    private var recenterPending = true
    private var lastTimestamp: TimeInterval = 0
    private var lastUI: TimeInterval = 0

    override init() {
        super.init()
        // Snapshot runs come from a shell, where TCC would attribute motion access to the
        // terminal and abort the process.
        guard !CommandLine.arguments.contains("--snapshot"), !CommandLine.arguments.contains("--bench") else { return }
        guard #available(macOS 14, *) else { return }
        link = MotionLink(owner: self)
        denied = Self.authorization == .denied || Self.authorization == .restricted
        deferred = Self.authorization == .notDetermined && Onboarding.pending
        start()
    }

    var available: Bool {
        guard #available(macOS 14, *), let manager else { return false }
        return manager.isDeviceMotionAvailable
    }

    static var authorization: CMAuthorizationStatus {
        guard #available(macOS 14, *) else { return .restricted }
        return CMHeadphoneMotionManager.authorizationStatus()
    }

    /// Starts tracking now, which makes the system ask for motion access the first time.
    func requestAccess() {
        deferred = false
        if !enabled { enabled = true } else { start() }
        if Self.authorization == .denied || Self.authorization == .restricted { denied = true }
    }

    func start() {
        guard #available(macOS 14, *), let manager else { return }
        guard enabled, !deferred, manager.isDeviceMotionAvailable, !manager.isDeviceMotionActive else { return }
        manager.startDeviceMotionUpdates(to: queue) { [weak self] motion, error in
            guard let self else { return }
            guard let motion else {
                if error != nil {
                    let status = Self.authorization
                    DispatchQueue.main.async {
                        if status == .denied || status == .restricted { self.denied = true }
                    }
                }
                return
            }
            self.process(motion)
        }
    }

    func stop() {
        if #available(macOS 14, *) { manager?.stopDeviceMotionUpdates() }
        queue.addOperation { [weak self] in
            self?.recenterPending = true
            self?.onSample?(Sample(q: simd_quatf(ix: 0, iy: 0, iz: 0, r: 1), w: .zero, age: 0))
        }
        active = false
        yaw = 0
        pitch = 0
        roll = 0
    }

    /// Makes the current facing direction "forward" in the scene (yaw only: up stays up).
    func recenter() {
        queue.addOperation { [weak self] in self?.recenterPending = true }
    }

    fileprivate func didConnect() {
        DispatchQueue.main.async {
            self.connected = true
            self.start()
        }
    }

    fileprivate func didDisconnect() {
        queue.addOperation { [weak self] in
            self?.recenterPending = true
            self?.onSample?(Sample(q: simd_quatf(ix: 0, iy: 0, iz: 0, r: 1), w: .zero, age: 0))
        }
        DispatchQueue.main.async {
            self.connected = false
            self.active = false
        }
    }

    // MARK: Motion queue

    /// Device frame -> head frame (engine axes), as a rotation about the vertical.
    private var deviceToHead: simd_quatf {
        // rightForwardUp: device x (right) -> head -y, device y (forward) -> head +x: Rz(-90 deg).
        // leftBackUp: device x (left) -> head +y, device y (back) -> head -x: Rz(+90 deg).
        let angle: Float = axes == .rightForwardUp ? -.pi / 2 : .pi / 2
        return simd_quatf(angle: angle, axis: SIMD3(0, 0, 1))
    }

    private func process(_ motion: CMDeviceMotion) {
        let a = motion.attitude.quaternion
        // Device -> CoreMotion reference frame (z vertical, x/y arbitrary horizontal).
        let qDev = simd_quatf(ix: Float(a.x), iy: Float(a.y), iz: Float(a.z), r: Float(a.w))
        let m = deviceToHead
        // Head -> reference: q_dev * m^-1 (a head-frame vector first goes to device axes).
        let qHeadRef = simd_normalize(qDev * m.inverse)
        let forward = qHeadRef.act(SIMD3<Float>(1, 0, 0))
        let currentHeading = atan2(forward.y, forward.x)

        let dt = lastTimestamp > 0 ? min(max(motion.timestamp - lastTimestamp, 0), 0.2) : 0
        lastTimestamp = motion.timestamp

        let r = motion.rotationRate
        let wDev = SIMD3<Float>(Float(r.x), Float(r.y), Float(r.z))
        let w = m.act(wDev)

        if recenterPending {
            heading = currentHeading
            recenterPending = false
        } else if driftCorrection {
            let rel = wrap(currentHeading - heading)
            if abs(rel) < 20 * .pi / 180, simd_length(w) < 0.15 {
                let step = Float(0.5 * .pi / 180 * dt)
                heading = wrap(heading + max(-step, min(step, rel)))
            }
        }
        // World = reference rotated so that "forward" at recentre is +x.
        let qHead = simd_normalize(simd_quatf(angle: -heading, axis: SIMD3(0, 0, 1)) * qHeadRef)
        let age = Float(max(0, ProcessInfo.processInfo.systemUptime - motion.timestamp))
        onSample?(Sample(q: qHead, w: w, age: min(age, 0.5)))

        // UI at <= 30 Hz on the main thread.
        let now = motion.timestamp
        if now - lastUI > 1.0 / 30 {
            lastUI = now
            let (y, p, rl) = Self.euler(qHead)
            DispatchQueue.main.async {
                self.yaw = Double(y)
                self.pitch = Double(p)
                self.roll = Double(rl)
                self.lastSampleTime = ProcessInfo.processInfo.systemUptime
                if self.denied { self.denied = false }
                if !self.active { self.active = true }
            }
        }
    }

    /// Engine convention (Mat3::from_head): R = Rz(yaw) Ry(-pitch) Rx(roll).
    static func euler(_ q: simd_quatf) -> (Float, Float, Float) {
        let m = simd_matrix3x3(q) // columns
        let r00 = m.columns.0.x, r10 = m.columns.0.y, r20 = m.columns.0.z
        let r21 = m.columns.1.z, r22 = m.columns.2.z
        return (atan2(r10, r00), asin(max(-1, min(1, r20))), atan2(r21, r22))
    }

    private func wrap(_ a: Float) -> Float {
        var x = a
        while x > .pi { x -= 2 * .pi }
        while x < -.pi { x += 2 * .pi }
        return x
    }
}

/// Owns the motion manager and forwards its delegate calls (the manager and its delegate
/// protocol are macOS 14+, the tracker itself is not).
@available(macOS 14, *)
private final class MotionLink: NSObject, CMHeadphoneMotionManagerDelegate {
    let manager = CMHeadphoneMotionManager()
    weak var owner: HeadTracker?

    init(owner: HeadTracker) {
        self.owner = owner
        super.init()
        manager.delegate = self
    }

    func headphoneMotionManagerDidConnect(_ manager: CMHeadphoneMotionManager) { owner?.didConnect() }

    func headphoneMotionManagerDidDisconnect(_ manager: CMHeadphoneMotionManager) { owner?.didDisconnect() }
}
