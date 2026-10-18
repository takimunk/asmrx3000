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
    private var bag = Set<AnyCancellable>()

    init() {
        head.onPose = { [weak audio] yaw, pitch, roll in
            audio?.setHeadPose(yaw: Float(yaw), pitch: Float(pitch), roll: Float(roll))
        }
        // Re-render the menu bar label when playback state changes.
        audio.$isPlaying
            .removeDuplicates()
            .sink { [weak self] playing in
                self?.icon.active = playing
                self?.objectWillChange.send()
            }
            .store(in: &bag)
    }

    func togglePlay() {
        audio.isPlaying ? audio.pause() : audio.play()
    }
}
