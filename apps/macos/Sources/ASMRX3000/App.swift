import Combine
import SwiftUI

@main
struct ASMRX3000App: App {
    @StateObject private var model = AppModel()

    init() {
        if let i = CommandLine.arguments.firstIndex(of: "--snapshot"), i + 1 < CommandLine.arguments.count {
            Snapshot.run(path: CommandLine.arguments[i + 1])
            exit(0)
        }
    }

    var body: some Scene {
        MenuBarExtra {
            ConsoleView(console: model.console)
        } label: {
            Image(nsImage: MenuBarIcon.image(active: model.audio.isPlaying))
        }
        .menuBarExtraStyle(.window)
    }
}

/// Owns the long-lived pieces so they survive the popover closing.
final class AppModel: ObservableObject {
    let audio = AudioEngine()
    let head = HeadTracker()
    lazy var console = Console(model: self)
    private var bag = Set<AnyCancellable>()

    init() {
        head.onPose = { [weak audio] yaw, pitch, roll in
            audio?.setHeadPose(yaw: Float(yaw), pitch: Float(pitch), roll: Float(roll))
        }
        // Re-render the menu bar label when playback state changes.
        audio.$isPlaying
            .removeDuplicates()
            .sink { [weak self] _ in self?.objectWillChange.send() }
            .store(in: &bag)
    }

    func togglePlay() {
        audio.isPlaying ? audio.pause() : audio.play()
    }
}
