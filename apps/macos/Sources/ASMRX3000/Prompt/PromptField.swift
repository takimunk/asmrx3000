import SwiftUI

/// Sends prompts to the provider and applies the replies; keeps undo patches for ⌘Z; records
/// the voice and turns it into a prompt (fal.ai Whisper).
@MainActor
final class PromptController: ObservableObject {
    enum Status: Equatable {
        case idle
        case thinking
        /// The model's one-line explanation of what changed.
        case said(String)
        /// An answer about the scene (may be two or three lines).
        case answer(String)
        /// Nothing changed, on purpose or because nothing matched: the reason.
        case hint(String)
        case failed(String)
        /// The microphone is on.
        case recording
        /// The recording is with Whisper.
        case transcribing

        var text: String {
            switch self {
            case .idle: return ""
            case .thinking: return "Слушаю…"
            case .recording: return "Говорите…"
            case .transcribing: return "Распознаю речь…"
            case let .said(s), let .answer(s), let .hint(s), let .failed(s): return s
            }
        }

        var busy: Bool { self == .thinking || self == .transcribing }
    }

    @Published var text = ""
    @Published private(set) var status: Status = .idle
    @Published private(set) var canUndo = false

    private let audio: AudioEngine
    private var undo: [Data] = []
    private var task: Task<Void, Never>?
    private let recorder = VoiceRecorder()
    /// Upper bound for one request; the field quietly gives up after it.
    nonisolated static let timeout: TimeInterval = 10
    nonisolated static let voiceTimeout: TimeInterval = 30

    init(audio: AudioEngine) {
        self.audio = audio
        recorder.onLimit = { [weak self] in Task { @MainActor in self?.stopVoice() } }
    }

    func send() {
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !words.isEmpty, status != .thinking, status != .recording else { return }
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
                var reply = try await withTimeout(Self.timeout) { try await provider.complete(request) }
                guard let self, !Task.isCancelled else { return }
                var result = self.apply(reply)
                // A model that writes keys gets one chance to fix what the core rejected.
                if provider.repairs, reply.kind == .patch, Self.appliedCount(result) == 0,
                   let errors = result["errors"] as? [String], !errors.isEmpty {
                    var retry = request
                    retry.feedback = "Your previous patch was rejected: " + errors.joined(separator: "; ") + ". Use only the listed keys and preset names."
                    let fixed = retry
                    reply = try await withTimeout(Self.timeout) { try await provider.complete(fixed) }
                    guard !Task.isCancelled else { return }
                    result = self.apply(reply)
                }
                self.status = Self.line(for: reply, result: result)
                if case .failed = self.status {} else { self.text = "" }
            } catch {
                guard let self, !Task.isCancelled else { return }
                let e = PromptError.from(error)
                NSLog("ASMR X3000: prompt via %@ failed: %@", provider.name, String(describing: e))
                self.status = .failed(e.errorDescription ?? "")
            }
        }
    }

    /// Applies a reply's patch (also used directly by --snapshot); returns the core's result.
    @discardableResult
    func apply(_ reply: PromptReply) -> [String: Any] {
        let result = audio.applyPatch(reply.patch)
        if let u = result["undo"], let d = try? JSONSerialization.data(withJSONObject: u), Self.appliedCount(result) > 0 {
            undo.append(d)
            if undo.count > 32 { undo.removeFirst() }
        }
        canUndo = !undo.isEmpty
        if let errors = result["errors"] as? [String], !errors.isEmpty {
            NSLog("ASMR X3000: patch errors: %@", errors.joined(separator: "; "))
        }
        status = Self.line(for: reply, result: result)
        return result
    }

    static func appliedCount(_ result: [String: Any]) -> Int {
        ((result["applied"] as? [Any])?.count ?? 0) + ((result["moved"] as? [Any])?.count ?? 0)
    }

    /// What the line under the field says after a reply: what changed, the answer, why
    /// nothing changed, or what the core rejected (in its own words).
    nonisolated static func line(for reply: PromptReply, result: [String: Any]) -> Status {
        let errors = result["errors_ru"] as? [String] ?? (result["errors"] as? [String] ?? [])
        let applied = ((result["applied"] as? [Any])?.count ?? 0) + ((result["moved"] as? [Any])?.count ?? 0)
        switch reply.kind {
        case .answer:
            return .answer(reply.explanation.isEmpty ? "Не могу описать сцену." : reply.explanation)
        case .nothing:
            return .hint(reply.explanation.isEmpty ? "Менять нечего." : reply.explanation)
        case .unmatched:
            return .hint(reply.explanation.isEmpty ? "Не понял, что изменить. Скажите иначе: «тише река», «ночь», «в тайгу»." : reply.explanation)
        case .patch:
            if applied == 0 {
                return errors.isEmpty ? .hint("Ничего не изменилось.") : .failed(PromptError.rejected(errors).errorDescription ?? "")
            }
            let said = reply.explanation.isEmpty ? "Готово." : reply.explanation
            return errors.isEmpty ? .said(said) : .said(said + " Не вышло: " + errors[0] + ".")
        }
    }

    /// ⌘Z: restores what the last prompt changed.
    func undoLast() {
        guard let patch = undo.popLast() else { return }
        task?.cancel()
        audio.applyPatch(patch)
        canUndo = !undo.isEmpty
        status = .said("Как было.")
    }

    // MARK: Voice

    func toggleVoice() {
        if status == .recording { stopVoice() } else { startVoice() }
    }

    func startVoice() {
        guard !status.busy, status != .recording else { return }
        guard FalWhisper.storedKey != nil else {
            status = .failed(VoiceError.noKey.errorDescription ?? "")
            return
        }
        status = .recording
        let recorder = recorder
        Task { [weak self] in
            guard await VoiceRecorder.permission() else {
                self?.status = .failed(VoiceError.denied.errorDescription ?? "")
                return
            }
            do {
                try await Task.detached { try recorder.start() }.value
            } catch {
                self?.status = .failed(VoiceError.from(error).errorDescription ?? "")
            }
        }
    }

    func stopVoice() {
        guard status == .recording, let key = FalWhisper.storedKey else { return }
        status = .transcribing
        let recorder = recorder
        task?.cancel()
        task = Task { [weak self] in
            do {
                let file = try await recorder.stop()
                defer { try? FileManager.default.removeItem(at: file) }
                let words = try await withTimeout(Self.voiceTimeout) { try await FalWhisper(key: key).transcribe(file) }
                guard let self, !Task.isCancelled else { return }
                self.text = words
                self.status = .idle
                self.send()
            } catch {
                guard let self, !Task.isCancelled else { return }
                self.status = .failed(VoiceError.from(error).errorDescription ?? "")
            }
        }
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

/// The prompt: a framed field that starts with "X3000" of the menu bar mark. Enter sends; the
/// microphone button records the voice (click to start and stop, or hold while speaking;
/// ⌥Space). The line under it says what happened (the model's answer, that it is listening,
/// or what went wrong).
struct PromptField: View {
    @ObservedObject var prompt: PromptController
    let stage: Stage
    @FocusState private var focused: Bool
    @State private var hoveringSend = false
    @State private var hoveringMic = false
    @State private var dots = 0
    @State private var pressStart: Date?
    @State private var stopOnRelease = false
    static let placeholder = "Скажите, каким быть лесу…"
    static let example = "Например: тише река, ночь, перенеси меня в горы"
    private let tick = Timer.publish(every: 0.35, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: Grid.x1 + 2) {
            HStack(spacing: Grid.x2 + 2) {
                PromptMark(lit: focused || prompt.status.busy || prompt.status == .recording, blinking: focused && !Motion.reduced)
                    .accessibilityHidden(true)
                if IconView.still {
                    // Snapshots cannot draw AppKit fields; the placeholder in its place.
                    Text(prompt.text.isEmpty ? Self.placeholder : prompt.text)
                        .font(Ink.text(Ink.Size.prompt))
                        .foregroundStyle(prompt.text.isEmpty ? Ink.graphite : Ink.paper)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    TextField("", text: $prompt.text,
                              prompt: Text(prompt.status == .recording ? "Говорите…" : Self.placeholder)
                                .font(Ink.text(Ink.Size.prompt)).foregroundColor(Ink.graphite))
                        .textFieldStyle(.plain)
                        .font(Ink.text(Ink.Size.prompt))
                        .foregroundStyle(Ink.paper)
                        .focused($focused)
                        .onSubmit { prompt.send() }
                        .onExitCommand { focused = false; stage.focusScene?() }
                        .disabled(prompt.status.busy)
                        .accessibilityLabel("Запрос")
                }
                mic
                send
            }
            .padding(.leading, Grid.x3)
            .padding(.trailing, Grid.x1)
            .frame(height: 40)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Ink.raised))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(prompt.status == .recording ? Ink.violet : (focused ? Ink.paper.opacity(0.55) : Ink.hairline), lineWidth: 1))
            .contentShape(Rectangle())
            .onTapGesture { focused = true }

            statusLine
                .padding(.horizontal, Grid.x3)
                .frame(minHeight: 15, alignment: .leading)
        }
        .background {
            // ⌘Z undoes the last prompt; ⌥Space starts / stops the voice.
            Button("Отменить запрос") { prompt.undoLast() }
                .keyboardShortcut("z", modifiers: .command)
                .disabled(!prompt.canUndo)
                .opacity(0)
                .accessibilityHidden(true)
            Button("Голосом") { prompt.toggleVoice() }
                .keyboardShortcut(.space, modifiers: .option)
                .opacity(0)
                .accessibilityHidden(true)
        }
        .onReceive(tick) { _ in if prompt.status.busy || prompt.status == .recording { dots = (dots + 1) % 4 } }
        .onAppear { stage.focusPrompt = { focused = true } }
    }

    /// Microphone: click to start, click again to send; or hold while speaking and let go.
    private var mic: some View {
        let recording = prompt.status == .recording
        let lit = recording || hoveringMic
        return MicGlyph(color: recording ? Ink.panel : (lit ? Ink.paper : Ink.graphite))
            .frame(width: 32, height: 32)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous)
                .fill(recording ? Ink.violet.opacity(dots % 2 == 0 || Motion.reduced ? 1 : 0.7) : (hoveringMic ? Ink.pressed : .clear)))
            .contentShape(Rectangle())
            .onHover { hoveringMic = $0 }
            .onLongPressGesture(minimumDuration: 0, maximumDistance: 60, perform: {}, onPressingChanged: { pressing in
                if pressing {
                    if prompt.status == .recording {
                        stopOnRelease = true
                    } else {
                        stopOnRelease = false
                        pressStart = Date()
                        prompt.startVoice()
                    }
                } else {
                    let held = Date().timeIntervalSince(pressStart ?? Date())
                    if stopOnRelease || (prompt.status == .recording && held > 0.45) { prompt.stopVoice() }
                    pressStart = nil
                    stopOnRelease = false
                }
            })
            .disabled(prompt.status.busy)
            .help(recording ? "Отпустите или нажмите ещё раз — отправить" : "Сказать голосом: нажмите или держите   ⌥Пробел")
            .accessibilityLabel(recording ? "Остановить запись" : "Сказать голосом")
            .accessibilityAddTraits(.isButton)
    }

    /// Return-key button: shown once there is something to send; three dots while it thinks.
    @ViewBuilder private var send: some View {
        if prompt.status.busy {
            HStack(spacing: 3) {
                ForEach(0..<3, id: \.self) { i in
                    Circle().fill(i < dots || Motion.reduced ? Ink.paper : Ink.ash).frame(width: 3, height: 3)
                }
            }
            .frame(width: 32, height: 32)
            .accessibilityLabel(prompt.status == .transcribing ? "Распознаю речь" : "Слушаю")
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
            .help("Отправить   Enter")
            .accessibilityLabel("Отправить")
        }
    }

    @ViewBuilder private var statusLine: some View {
        let dotted = String(repeating: ".", count: Motion.reduced ? 3 : dots)
        HStack(alignment: .firstTextBaseline, spacing: Grid.x2) {
            switch prompt.status {
            case .idle:
                Text(prompt.canUndo ? "⌘Z вернуть как было" : Self.example).foregroundStyle(Ink.ash)
            case .thinking:
                Text("Слушаю" + dotted).foregroundStyle(Ink.graphite)
            case .recording:
                Circle().fill(Ink.violet).frame(width: 7, height: 7)
                Text("Говорите" + dotted + "  отпустите или нажмите ещё раз").foregroundStyle(Ink.paper.opacity(0.85))
            case .transcribing:
                Text("Распознаю речь" + dotted).foregroundStyle(Ink.graphite)
            case let .said(s):
                Text(s).foregroundStyle(Ink.paper.opacity(0.85))
                    .truncationMode(.tail)
                    .help(s)
                Spacer(minLength: Grid.x2)
                if prompt.canUndo { Text("⌘Z отменить").foregroundStyle(Ink.ash).fixedSize() }
            case let .answer(s):
                Text(s).foregroundStyle(Ink.paper.opacity(0.9))
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
                    .help(s)
            case let .hint(s):
                Text("?")
                    .foregroundStyle(Ink.panel)
                    .frame(width: 13, height: 13)
                    .background(Circle().fill(Ink.graphite))
                Text(s).foregroundStyle(Ink.paper.opacity(0.85)).help(s)
            case let .failed(s):
                Text("!")
                    .foregroundStyle(Ink.panel)
                    .frame(width: 13, height: 13)
                    .background(Circle().fill(Ink.paper))
                Text(s).foregroundStyle(Ink.paper).help(s)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(prompt.status.isAnswer ? 3 : 1)
        .accessibilityElement(children: .combine)
    }
}

private extension PromptController.Status {
    var isAnswer: Bool { if case .answer = self { return true } else { return false } }
}

/// A microphone in the pixels of the mark: capsule, cradle, stem and foot.
struct MicGlyph: View {
    var color: Color

    var body: some View {
        Canvas { ctx, size in
            let px: CGFloat = 1.5
            let rows = [
                "..###..",
                ".#...#.",
                ".#...#.",
                ".#...#.",
                ".#...#.",
                "#.###.#",
                "#.....#",
                ".#...#.",
                "..###..",
                "...#...",
                ".#####.",
            ]
            let w = CGFloat(rows[0].count) * px, h = CGFloat(rows.count) * px
            let x0 = ((size.width - w) / 2).rounded(), y0 = ((size.height - h) / 2).rounded()
            for (r, row) in rows.enumerated() {
                for (c, ch) in row.enumerated() where ch == "#" {
                    ctx.fill(Path(CGRect(x: x0 + CGFloat(c) * px, y: y0 + CGFloat(r) * px, width: px, height: px)), with: .color(color))
                }
            }
        }
        .accessibilityHidden(true)
    }
}

/// "X3000" in the menu bar mark's pixels (the lower line of "ASMR / X3000", same sheared
/// bitmap face): the prompt is where you talk to the scene. A cursor after it blinks while the
/// field has focus.
struct PromptMark: View {
    var lit: Bool
    var blinking: Bool
    @State private var on = true
    private let blink = Timer.publish(every: 0.55, on: .main, in: .common).autoconnect()

    /// Letters X, 3, 0, 0, 0 of the mark (y up, points) and their bounds.
    private static let runs: [CGRect] = MenuBarGlyphs.letters.suffix(5).flatMap { $0 }
    private static let bounds: CGRect = runs.reduce(CGRect.null) { $0.union($1) }
    static let px: CGFloat = 2
    static let size = CGSize(width: bounds.width * px + 3 * px, height: bounds.height * px)

    var body: some View {
        Canvas { ctx, _ in
            let px = Self.px, b = Self.bounds
            let ink = lit ? Ink.paper : Ink.graphite
            for r in Self.runs {
                // Flip: the glyphs are y up.
                let rect = CGRect(x: (r.minX - b.minX) * px, y: (b.maxY - r.maxY) * px, width: r.width * px, height: r.height * px)
                ctx.fill(Path(rect), with: .color(ink))
            }
            if blinking && on {
                ctx.fill(Path(CGRect(x: b.width * px + px, y: b.height * px - px, width: 2 * px, height: px)), with: .color(ink))
            }
        }
        .frame(width: Self.size.width, height: Self.size.height)
        .onReceive(blink) { _ in on = blinking ? !on : true }
    }
}
