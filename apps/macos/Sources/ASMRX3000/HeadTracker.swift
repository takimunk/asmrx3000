import CoreMotion
import Foundation

/// Head orientation from AirPods (Pro / Max / 3rd gen+) and supported Beats via CoreMotion.
/// Only the pose is taken from Apple; all spatial rendering happens in asmr-core.
final class HeadTracker: NSObject, ObservableObject, CMHeadphoneMotionManagerDelegate {
    @Published private(set) var connected = false
    @Published private(set) var active = false
    @Published var enabled = true {
        didSet { enabled ? start() : stop() }
    }

    /// Relative to the last recenter, radians. Engine convention: yaw + = left, pitch + = up,
    /// roll + = right ear down.
    private(set) var yaw: Double = 0
    private(set) var pitch: Double = 0
    private(set) var roll: Double = 0

    var onPose: ((Double, Double, Double) -> Void)?

    // CoreMotion's headphone axes vs. ours. Flip a sign here if a direction feels inverted.
    private static let yawSign = 1.0
    private static let pitchSign = 1.0
    private static let rollSign = 1.0

    private let manager = CMHeadphoneMotionManager()
    private var rawYaw = 0.0
    private var yawOffset = 0.0

    override init() {
        super.init()
        // Snapshot runs come from a shell, where TCC would attribute motion access to the
        // terminal and abort the process.
        guard !CommandLine.arguments.contains("--snapshot"), !CommandLine.arguments.contains("--bench") else { return }
        manager.delegate = self
        start()
    }

    var available: Bool { manager.isDeviceMotionAvailable }

    func start() {
        guard enabled, manager.isDeviceMotionAvailable, !manager.isDeviceMotionActive else { return }
        manager.startDeviceMotionUpdates(to: .main) { [weak self] motion, _ in
            guard let self, let motion else { return }
            let a = motion.attitude
            self.rawYaw = a.yaw
            self.yaw = Self.wrap(a.yaw - self.yawOffset) * Self.yawSign
            self.pitch = a.pitch * Self.pitchSign
            self.roll = a.roll * Self.rollSign
            if !self.active { self.active = true }
            self.onPose?(self.yaw, self.pitch, self.roll)
        }
    }

    func stop() {
        manager.stopDeviceMotionUpdates()
        active = false
        yaw = 0
        pitch = 0
        roll = 0
        onPose?(0, 0, 0)
    }

    /// Makes the current facing direction "forward" in the scene.
    func recenter() {
        yawOffset = rawYaw
    }

    func headphoneMotionManagerDidConnect(_ manager: CMHeadphoneMotionManager) {
        DispatchQueue.main.async {
            self.connected = true
            self.start()
        }
    }

    func headphoneMotionManagerDidDisconnect(_ manager: CMHeadphoneMotionManager) {
        DispatchQueue.main.async {
            self.connected = false
            self.active = false
            self.onPose?(0, 0, 0)
        }
    }

    private static func wrap(_ a: Double) -> Double {
        var x = a
        while x > .pi { x -= 2 * .pi }
        while x < -.pi { x += 2 * .pi }
        return x
    }
}
