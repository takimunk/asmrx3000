import Foundation

/// First launch: why we want the AirPods' motion, the permission, the head that follows yours,
/// "look at the centre of the screen" (sets the audio's forward), the screen's size and
/// distance, then the forest starts. Shown until finished once ("onboarding.done"); again with
/// `--onboarding` or from the popover (right click on "Голова" or on recenter).
enum Onboarding {
    static let doneKey = "onboarding.done"
    static let forced = CommandLine.arguments.contains("--onboarding")
    /// Command-line runs that must never open a window or ask for permissions.
    static let headless = ["--snapshot", "--bench", "--prompt-live", "--voice-check", "--menubar-frames"]
        .contains { CommandLine.arguments.contains($0) }

    static var done: Bool {
        get { UserDefaults.standard.bool(forKey: doneKey) }
        set { UserDefaults.standard.set(newValue, forKey: doneKey) }
    }

    /// Should run at this launch (HeadTracker holds back its permission request until then).
    static var pending: Bool { !headless && (forced || !done) }
}
