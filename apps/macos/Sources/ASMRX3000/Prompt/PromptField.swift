import SwiftUI

/// Sends prompts to the provider and applies the replies; keeps undo patches for ⌘Z.
@MainActor
final class PromptController: ObservableObject {
    enum Status: Equatable {
        case idle
        case thinking
        /// The model's one-line explanation.
        case said(String)
        case failed(String)
    }

    @Published var text = ""
    @Published private(set) var status: Status = .idle
    @Published private(set) var canUndo = false

    private let audio: AudioEngine
    private var undo: [Data] = []
    private var task: Task<Void, Never>?
    /// Upper bound for one request; the field quietly gives up after it.
    static let timeout: TimeInterval = 10

    init(audio: AudioEngine) {
        self.audio = audio
    }

    func send() {
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !words.isEmpty, status != .thinking else { return }
        guard let provider = Providers.current() else {
            status = .failed(PromptError.noProvider.errorDescription ?? "")
            return
        }
        let request: PromptRequest
        do {
            request = try PromptRequest(text: words, promptJSON: audio.promptJSON(), state: audio.stateJSON())
        } catch {
            status = .failed(PromptError.from(error).errorDescription ?? "")
            return
        }
        status = .thinking
        task?.cancel()
        task = Task { [weak self] in
            do {
                let reply = try await withTimeout(Self.timeout) { try await provider.complete(request) }
                guard let self, !Task.isCancelled else { return }
                self.apply(reply)
                self.text = ""
            } catch {
                guard let self, !Task.isCancelled else { return }
                let e = PromptError.from(error)
                NSLog("ASMR X3000: prompt via %@ failed: %@", provider.name, String(describing: e))
                self.status = .failed(e.errorDescription ?? "")
            }
        }
    }

    /// Applies a reply's patch (also used directly by --snapshot).
    func apply(_ reply: PromptReply) {
        let result = audio.applyPatch(reply.patch)
        if let u = result["undo"], let d = try? JSONSerialization.data(withJSONObject: u),
           !((result["applied"] as? [Any])?.isEmpty ?? true) || !((result["moved"] as? [Any])?.isEmpty ?? true) {
            undo.append(d)
            if undo.count > 32 { undo.removeFirst() }
        }
        canUndo = !undo.isEmpty
        if let errors = result["errors"] as? [String], !errors.isEmpty {
            NSLog("ASMR X3000: patch errors: %@", errors.joined(separator: "; "))
        }
        status = .said(reply.explanation.isEmpty ? "Готово." : reply.explanation)
    }

    /// ⌘Z: restores what the last prompt changed.
    func undoLast() {
        guard let patch = undo.popLast() else { return }
        task?.cancel()
        audio.applyPatch(patch)
        canUndo = !undo.isEmpty
        status = .said("Как было.")
    }
}

private func withTimeout<T: Sendable>(_ seconds: TimeInterval, _ work: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await work() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(seconds * 1e9))
            throw PromptError.timeout
        }
        let first = try await group.next()!
        group.cancelAll()
        return first
    }
}

/// One engraved line: the request is written on it, the answer appears under it.
struct PromptField: View {
    @ObservedObject var prompt: PromptController
    @FocusState private var focused: Bool
    @State private var dots = 0
    static let placeholder = "Скажите, каким быть лесу…"
    private let tick = Timer.publish(every: 0.35, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Text("❧")
                    .font(Ink.text(13))
                    .foregroundStyle(focused ? Ink.paper : Ink.graphite)
                    .accessibilityHidden(true)
                if IconView.still {
                    // Snapshots cannot draw AppKit fields; the placeholder in its place.
                    Text(prompt.text.isEmpty ? Self.placeholder : prompt.text)
                        .font(Ink.italic(14))
                        .foregroundStyle(prompt.text.isEmpty ? Ink.graphite : Ink.paper)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    TextField("", text: $prompt.text, prompt: Text(Self.placeholder).font(Ink.italic(14)).foregroundColor(Ink.graphite))
                        .textFieldStyle(.plain)
                        .font(Ink.italic(14))
                        .foregroundStyle(Ink.paper)
                        .focused($focused)
                        .onSubmit { prompt.send() }
                        .disabled(prompt.status == .thinking)
                        .accessibilityLabel("Prompt")
                }
            }
            Rectangle()
                .fill(focused ? Ink.graphite : Ink.ash)
                .frame(height: 0.5)
            Text(line)
                .font(Ink.text(11.5))
                .foregroundStyle(Ink.graphite)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(height: 14, alignment: .leading)
                .accessibilityLabel(line)
        }
        .background {
            // ⌘Z undoes the last prompt.
            Button("Undo prompt") { prompt.undoLast() }
                .keyboardShortcut("z", modifiers: .command)
                .disabled(!prompt.canUndo)
                .opacity(0)
                .accessibilityHidden(true)
        }
        .onReceive(tick) { _ in if prompt.status == .thinking { dots = (dots + 1) % 4 } }
    }

    private var line: String {
        switch prompt.status {
        case .idle: return prompt.canUndo ? "⌘Z — вернуть как было." : ""
        case .thinking: return "слушаю" + String(repeating: ".", count: dots)
        case let .said(s): return prompt.canUndo ? s + "   ⌘Z — назад" : s
        case let .failed(s): return s
        }
    }
}
