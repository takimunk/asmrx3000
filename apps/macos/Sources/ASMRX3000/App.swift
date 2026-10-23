import AppKit
import Combine
import SwiftUI

// The menu bar item and its panel are AppKit (MenuBarController): SwiftUI's MenuBarExtra
// window does not show Metal views on macOS 27. The app has no SwiftUI scenes at all: an
// empty `Settings` scene, the old placeholder, opened as a blank window at every launch.
@main
enum ASMRX3000App {
    @MainActor
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        app.mainMenu = editMenu()
        handleArguments()
        app.run()
    }

    /// Without a main menu text fields lose ⌘C / ⌘V / ⌘Z (keys, the prompt): keep an Edit menu.
    @MainActor
    private static func editMenu() -> NSMenu {
        let main = NSMenu()
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        let item = NSMenuItem()
        item.submenu = edit
        main.addItem(NSMenuItem())
        main.addItem(item)
        return main
    }

    @MainActor
    private static func handleArguments() {
        Fonts.register()
        if let i = CommandLine.arguments.firstIndex(of: "--snapshot"), i + 1 < CommandLine.arguments.count {
            Snapshot.run(path: CommandLine.arguments[i + 1])
            exit(0)
        }
        if let i = CommandLine.arguments.firstIndex(of: "--prompt-live"), i + 1 < CommandLine.arguments.count {
            PromptCheck.run(Array(CommandLine.arguments[(i + 1)...]))
            exit(0)
        }
        if let i = CommandLine.arguments.firstIndex(of: "--voice-check") {
            VoiceCheck.run(i + 1 < CommandLine.arguments.count ? CommandLine.arguments[i + 1] : nil)
            exit(0)
        }
        if let i = CommandLine.arguments.firstIndex(of: "--menubar-frames"), i + 1 < CommandLine.arguments.count {
            MenuBarFrames.write(to: CommandLine.arguments[i + 1])
            exit(0)
        }
        if CommandLine.arguments.contains("--bench") {
            Bench.start()
        }
    }
}

/// Creates the model and the menu bar item once the app has launched.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var menuBar: MenuBarController?

    @MainActor
    func applicationDidFinishLaunching(_ notification: Notification) {
        menuBar = MenuBarController(model: AppModel())
    }
}

/// Owns the long-lived pieces so they survive the popover closing.
final class AppModel: ObservableObject {
    let audio = AudioEngine()
    let head = HeadTracker()
    let icon = MenuBarIconAnimator()
    /// The display's size and distance for the engine, kept current at every launch.
    let screens: ScreenWatcher
    @MainActor lazy var onboarding = OnboardingController(model: self)
    lazy var stage = Stage(audio: audio, head: head)
    @MainActor lazy var prompt = PromptController(audio: audio)
    private var bag = Set<AnyCancellable>()

    init() {
        screens = ScreenWatcher(audio: audio)
        // Straight from the motion queue: the engine side is lock-free.
        head.onSample = { [weak audio] sample in
            audio?.setHeadSample(sample)
        }
        // Re-render the menu bar label when playback state changes.
        audio.$isPlaying
            .removeDuplicates()
            .sink { [weak self] playing in
                self?.icon.playing = playing
                self?.objectWillChange.send()
            }
            .store(in: &bag)
        // A new accent: redraw the panel's views that use it (Ink.accent is read at render).
        Accent.shared.$hue
            .dropFirst()
            .removeDuplicates()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                guard let self else { return }
                MainActor.assumeIsolated {
                    self.objectWillChange.send()
                    self.prompt.objectWillChange.send()
                }
            }
            .store(in: &bag)
        // The popover is open exactly while the scene is on screen.
        stage.meters.$live
            .removeDuplicates()
            .sink { [weak self] open in self?.icon.open = open }
            .store(in: &bag)
        // --open-panel (development): open the menu bar panel right after launch.
        if CommandLine.arguments.contains("--open-panel") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                MainActor.assumeIsolated { self?.openPanel() }
            }
        }
        // First launch (or --onboarding): explain head tracking before the system asks for it.
        if Onboarding.pending {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                MainActor.assumeIsolated { self?.onboarding.show() }
            }
        }
    }

    /// Opens the menu bar panel (after onboarding, or with --open-panel).
    @MainActor
    func openPanel() {
        MenuBarController.shared?.open()
    }

    func togglePlay() {
        audio.isPlaying ? audio.pause() : audio.play()
    }
}
