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

/// The prompt: a framed field that starts with the "X_" of the menu bar mark. Enter sends; the
/// line under it says what happened (the model's answer, that it is listening, or what went wrong).
struct PromptField: View {
    @ObservedObject var prompt: PromptController
    let stage: Stage
    @FocusState private var focused: Bool
    @State private var hoveringSend = false
    @State private var dots = 0
    static let placeholder = "Скажите, каким быть лесу…"
    static let example = "Например: тише, ближе к ночи, дождь посильнее"
    private let tick = Timer.publish(every: 0.35, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: Grid.x1 + 2) {
            HStack(spacing: Grid.x2 + 2) {
                PromptMark(lit: focused || prompt.status == .thinking, blinking: focused && !Motion.reduced)
                    .accessibilityHidden(true)
                if IconView.still {
                    // Snapshots cannot draw AppKit fields; the placeholder in its place.
                    Text(prompt.text.isEmpty ? Self.placeholder : prompt.text)
                        .font(Ink.text(Ink.Size.prompt))
                        .foregroundStyle(prompt.text.isEmpty ? Ink.graphite : Ink.paper)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    TextField("", text: $prompt.text,
                              prompt: Text(Self.placeholder).font(Ink.text(Ink.Size.prompt)).foregroundColor(Ink.graphite))
                        .textFieldStyle(.plain)
                        .font(Ink.text(Ink.Size.prompt))
                        .foregroundStyle(Ink.paper)
                        .focused($focused)
                        .onSubmit { prompt.send() }
                        .onExitCommand { focused = false; stage.focusScene?() }
                        .disabled(prompt.status == .thinking)
                        .accessibilityLabel("Prompt")
                }
                send
            }
            .padding(.leading, Grid.x3)
            .padding(.trailing, Grid.x1)
            .frame(height: 40)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Ink.raised))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(focused ? Ink.paper.opacity(0.55) : Ink.hairline, lineWidth: 1))
            .contentShape(Rectangle())
            .onTapGesture { focused = true }

            statusLine
                .padding(.horizontal, Grid.x3)
                .frame(height: 15, alignment: .leading)
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
        .onAppear { stage.focusPrompt = { focused = true } }
    }

    /// Return-key button: shown once there is something to send; three dots while it thinks.
    @ViewBuilder private var send: some View {
        if prompt.status == .thinking {
            HStack(spacing: 3) {
                ForEach(0..<3, id: \.self) { i in
                    Circle().fill(i < dots || Motion.reduced ? Ink.paper : Ink.ash).frame(width: 3, height: 3)
                }
            }
            .frame(width: 32, height: 32)
            .accessibilityLabel("Listening")
        } else {
            let ready = !prompt.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            Button { prompt.send() } label: {
                Text("⏎")
                    .font(Ink.text(Ink.Size.prompt))
                    .foregroundStyle(ready ? (hoveringSend ? Ink.panel : Ink.paper) : Ink.ash)
                    .frame(width: 32, height: 32)
                    .background(RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(ready && hoveringSend ? Ink.paper : .clear))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .disabled(!ready)
            .onHover { hoveringSend = $0 }
            .help("Send   Enter")
            .accessibilityLabel("Send")
        }
    }

    @ViewBuilder private var statusLine: some View {
        HStack(spacing: Grid.x2) {
            switch prompt.status {
            case .idle:
                Text(prompt.canUndo ? "⌘Z вернуть как было" : Self.example).foregroundStyle(Ink.ash)
            case .thinking:
                Text("Слушаю" + String(repeating: ".", count: Motion.reduced ? 3 : dots)).foregroundStyle(Ink.graphite)
            case let .said(s):
                Text(s).foregroundStyle(Ink.paper.opacity(0.85))
                    .truncationMode(.tail)
                Spacer(minLength: Grid.x2)
                if prompt.canUndo { Text("⌘Z отменить").foregroundStyle(Ink.ash).fixedSize() }
            case let .failed(s):
                Text("!")
                    .foregroundStyle(Ink.panel)
                    .frame(width: 13, height: 13)
                    .background(Circle().fill(Ink.paper))
                Text(s).foregroundStyle(Ink.paper)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(1)
        .accessibilityElement(children: .combine)
    }
}

/// "X_" in the menu bar mark's pixels: the prompt is where you type to the forest. The cursor
/// blinks while the field has focus.
struct PromptMark: View {
    var lit: Bool
    var blinking: Bool
    @State private var on = true
    private let blink = Timer.publish(every: 0.55, on: .main, in: .common).autoconnect()

    var body: some View {
        Canvas { ctx, _ in
            let px: CGFloat = 1.5
            // X, 5 x 7, in a stair-step italic like the menu bar.
            let x = ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"]
            for (r, row) in x.enumerated() {
                let shift = CGFloat(6 - r) / 2 * px * 0.5
                for (c, ch) in row.enumerated() where ch == "#" {
                    ctx.fill(Path(CGRect(x: CGFloat(c) * px + shift, y: CGFloat(r) * px + 1, width: px, height: px)),
                             with: .color(lit ? Ink.paper : Ink.graphite))
                }
            }
            if on || !blinking {
                ctx.fill(Path(CGRect(x: 6 * px, y: 6 * px + 1, width: 4 * px, height: px)),
                         with: .color(lit ? Ink.paper : Ink.graphite))
            }
        }
        .frame(width: 16, height: 12)
        .onReceive(blink) { _ in on = blinking ? !on : true }
    }
}
