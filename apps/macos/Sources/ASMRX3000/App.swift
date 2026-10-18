import Combine
import SwiftUI

@main
struct ASMRX3000App: App {
    @StateObject private var model = AppModel()

    init() {
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

    var body: some Scene {
        MenuBarExtra {
            Popover(model: model)
        } label: {
            MenuBarLabel(icon: model.icon)
        }
        .menuBarExtraStyle(.window)
    }
}

/// Menu bar label; observes only the animator so animation frames do not re-render the app scene.
private struct MenuBarLabel: View {
    @ObservedObject var icon: MenuBarIconAnimator
    var body: some View { Image(nsImage: icon.image) }
}

/// Owns the long-lived pieces so they survive the popover closing.
final class AppModel: ObservableObject {
    let audio = AudioEngine()
    let head = HeadTracker()
    let icon = MenuBarIconAnimator()
    lazy var stage = Stage(audio: audio, head: head)
    @MainActor lazy var prompt = PromptController(audio: audio)
    private var bag = Set<AnyCancellable>()

    init() {
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
        // The popover is open exactly while the scene is on screen.
        stage.meters.$live
            .removeDuplicates()
            .sink { [weak self] open in self?.icon.open = open }
            .store(in: &bag)
    }

    func togglePlay() {
        audio.isPlaying ? audio.pause() : audio.play()
    }
}
