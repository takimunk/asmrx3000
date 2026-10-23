import AppKit
import CoreGraphics

/// The physical screen in front of the listener, for the engine's ASMR_PARAM_SCREEN_* (metres).
///
/// Size: CGDisplayScreenSize (EDID, millimetres); when a display reports nothing (some
/// adapters, AirPlay, virtual displays) it is guessed from its size in points: Apple's built-in
/// panels show about 0.2 mm per point at their default scaling, desktop monitors about 0.25.
/// Distance from the eyes: 0.6 m for a laptop's own panel, 0.75 m for an external display,
/// unless the user set one at the end of onboarding ("screenDistance.builtin" / ".external").
/// Elevation of the screen's centre seen from the eyes, degrees (− = below): −15° for a laptop's
/// panel, −5° for an external display, or as dragged in onboarding ("screenElevation.*").
struct ScreenGeometry: Equatable {
    var displayID: CGDirectDisplayID
    var width: Double
    var height: Double
    var distance: Double
    /// Degrees, − = below the eyes.
    var elevation: Double
    var builtIn: Bool
    /// The size came from the display itself, not from the guess.
    var measured: Bool

    static let defaultDistanceBuiltIn = 0.6
    static let defaultDistanceExternal = 0.75
    static let distanceRange = 0.3...2.0
    static let defaultElevationBuiltIn = -15.0
    static let defaultElevationExternal = -5.0
    static let elevationRange = -45.0...30.0

    /// Diagonal in inches, for the summary ("14.2″").
    var diagonalInches: Double { (width * width + height * height).squareRoot() / 0.0254 }

    static func displayID(of screen: NSScreen) -> CGDirectDisplayID? {
        (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber).map { CGDirectDisplayID($0.uint32Value) }
    }

    static func of(_ screen: NSScreen) -> ScreenGeometry {
        let id = displayID(of: screen) ?? CGMainDisplayID()
        let builtIn = CGDisplayIsBuiltin(id) != 0
        let mm = CGDisplayScreenSize(id)
        let points = screen.frame.size
        var w = Double(mm.width) / 1000, h = Double(mm.height) / 1000
        // EDID sizes of 0, or nonsense (projectors report 160 x 90 cm "screens", some adapters
        // 1 x 1 mm): keep anything a person could sit in front of, otherwise guess.
        let measured = w > 0.1 && h > 0.06 && w < 3 && h < 2
        if !measured {
            let mmPerPoint = builtIn ? 0.2 : 0.25
            w = Double(points.width) * mmPerPoint / 1000
            h = Double(points.height) * mmPerPoint / 1000
        } else if points.width > 0, points.height > 0 {
            // CGDisplayScreenSize is for the panel's native orientation; follow a rotated display.
            if (points.width > points.height) != (w > h) { swap(&w, &h) }
        }
        return ScreenGeometry(displayID: id, width: w, height: h, distance: distance(builtIn: builtIn),
                              elevation: elevation(builtIn: builtIn),
                              builtIn: builtIn, measured: measured)
    }

    static func distanceKey(builtIn: Bool) -> String { builtIn ? "screenDistance.builtin" : "screenDistance.external" }

    static func distance(builtIn: Bool) -> Double {
        let stored = UserDefaults.standard.double(forKey: distanceKey(builtIn: builtIn))
        if stored > 0 { return min(max(stored, distanceRange.lowerBound), distanceRange.upperBound) }
        return builtIn ? defaultDistanceBuiltIn : defaultDistanceExternal
    }

    static func setDistance(_ d: Double, builtIn: Bool) {
        UserDefaults.standard.set(min(max(d, distanceRange.lowerBound), distanceRange.upperBound), forKey: distanceKey(builtIn: builtIn))
    }

    static func elevationKey(builtIn: Bool) -> String { builtIn ? "screenElevation.builtin" : "screenElevation.external" }

    static func elevation(builtIn: Bool) -> Double {
        let d = UserDefaults.standard
        guard d.object(forKey: elevationKey(builtIn: builtIn)) != nil else {
            return builtIn ? defaultElevationBuiltIn : defaultElevationExternal
        }
        return min(max(d.double(forKey: elevationKey(builtIn: builtIn)), elevationRange.lowerBound), elevationRange.upperBound)
    }

    static func setElevation(_ e: Double, builtIn: Bool) {
        UserDefaults.standard.set(min(max(e, elevationRange.lowerBound), elevationRange.upperBound), forKey: elevationKey(builtIn: builtIn))
    }

    /// The display the listener faces: the one calibrated in onboarding while it is connected,
    /// otherwise the main screen (the one with the menu bar or the key window).
    static var preferredScreen: NSScreen? {
        let calibrated = UserDefaults.standard.object(forKey: "screen.displayID") as? NSNumber
        if let calibrated, let s = NSScreen.screens.first(where: { displayID(of: $0) == calibrated.uint32Value }) {
            return s
        }
        return NSScreen.main ?? NSScreen.screens.first
    }

    static func remember(_ screen: NSScreen) {
        if let id = displayID(of: screen) { UserDefaults.standard.set(NSNumber(value: id), forKey: "screen.displayID") }
    }
}

/// Keeps the engine's screen parameters in step with the displays: at launch, whenever a
/// display is connected, removed or changes resolution, and after onboarding.
final class ScreenWatcher {
    private weak var audio: AudioEngine?
    private var observer: NSObjectProtocol?
    private(set) var current: ScreenGeometry?

    init(audio: AudioEngine) {
        self.audio = audio
        observer = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in self?.apply() }
        apply()
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    /// Re-reads the preferred display (or `screen`, e.g. the one onboarding's window is on).
    @discardableResult
    func apply(_ screen: NSScreen? = nil) -> ScreenGeometry? {
        guard let s = screen ?? ScreenGeometry.preferredScreen else { return nil }
        let g = ScreenGeometry.of(s)
        current = g
        audio?.setScreen(width: Float(g.width), height: Float(g.height), distance: Float(g.distance),
                         elevation: Float(g.elevation))
        return g
    }
}
