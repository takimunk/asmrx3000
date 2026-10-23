import AVFoundation
import AppKit
import SwiftUI

// The few settings the app has: the accent, the two API keys (TypeSafe AI for the prompt,
// fal.ai for voice input) and the microphone permission. The same rows sit in the onboarding's
// last step and, folded, at the bottom of the panel.

/// One API key: a secure field and "save" while none is set; "saved" with "change" and
/// "remove" once it is. Saved to the login keychain (Keychain.save); the prompt and the voice
/// read it on their next use, no relaunch.
struct KeyRow: View {
    let account: Keychain.Account
    let name: LocalizedStringKey
    let placeholder: LocalizedStringKey
    var nameWidth: CGFloat = 112
    /// Why the key is needed: a "?" after the name, tooltip on hover, popover on click.
    var help: LocalizedStringKey?
    @ObservedObject private var accent = Accent.shared
    @State private var value = ""
    @State private var editing = false
    @State private var stored = false
    @State private var problem: String?
    @State private var justSaved = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: Grid.x2) {
                HStack(spacing: Grid.x1) {
                    Text(name).foregroundStyle(Ink.graphite)
                    if let help { HelpMark(text: help) }
                }
                .frame(width: nameWidth, alignment: .leading)
                if stored, !editing {
                    savedState
                } else {
                    field
                }
            }
            .frame(height: 28)
            if let problem {
                HStack(spacing: Grid.x1) {
                    Text("!").foregroundStyle(Ink.paper)
                    Text(verbatim: problem).foregroundStyle(Ink.graphite)
                }
                .font(Ink.text(Ink.Size.caption))
                .padding(.leading, nameWidth + Grid.x2)
            }
        }
        .font(Ink.text(Ink.Size.body))
        .onAppear { stored = Keychain.has(account) }
    }

    @ViewBuilder private var savedState: some View {
        if Keychain.fromEnvironment(account) {
            Text("set by \(account.env)").foregroundStyle(Ink.ash)
            Spacer(minLength: 0)
        } else {
            HStack(spacing: Grid.x1) {
                Circle().fill(Ink.accent).frame(width: 5, height: 5)
                (justSaved ? Text("saved") : Text("added")).foregroundStyle(Ink.paper)
            }
            Spacer(minLength: Grid.x2)
            TextButton(title: "change") { editing = true; focused = true }
            TextButton(title: "remove") {
                Keychain.remove(account)
                stored = Keychain.has(account)
                justSaved = false
            }
        }
    }

    private var field: some View {
        HStack(spacing: Grid.x1) {
            SecureField("", text: $value, prompt: Text(placeholder).foregroundColor(Ink.ash))
                .textFieldStyle(.plain)
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(Ink.paper)
                .focused($focused)
                .onSubmit(save)
                .onChange(of: value) { _ in problem = nil }
                .padding(.horizontal, Grid.x2)
                .frame(height: 26)
                .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(Ink.raised))
                .overlay(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous)
                    .strokeBorder(focused ? Ink.paper.opacity(0.55) : Ink.hairline, lineWidth: 1))
            if !value.isEmpty {
                TextButton(title: "save", strong: true, action: save)
            } else if editing {
                TextButton(title: "cancel") { editing = false; problem = nil }
            }
        }
    }

    /// What each key is for (Prompt/TypeSafeProvider.swift, Prompt/VoiceInput.swift).
    static let typesafeHelp: LocalizedStringKey = "TypeSafe AI powers the text prompt. What you type (\"quieter river, night\") is sent with the scene's current state, and it picks which sound and world controls to change and how fast. It also answers questions about what is around you. Without this key the prompt does nothing."
    static let falHelp: LocalizedStringKey = "fal.ai powers voice input. Click or hold the microphone in the prompt and speak (Russian, up to 30 s); the recording goes to fal.ai's Whisper, which turns it into text, and the text is sent as a prompt, so voice also needs the TypeSafe AI key. Audio leaves the Mac only while you record."

    private func save() {
        if let p = Keychain.problem(value, account: account) {
            problem = p
            return
        }
        guard Keychain.save(value, account: account) else {
            problem = "could not save to the keychain"
            return
        }
        value = ""
        editing = false
        problem = nil
        stored = true
        justSaved = true
    }
}

/// Why a key is needed: a small "?" in a ring; the text as a tooltip on hover and in a popover
/// on click.
struct HelpMark: View {
    let text: LocalizedStringKey
    @State private var hovering = false
    @State private var shown = false

    var body: some View {
        Button { shown.toggle() } label: {
            Text(verbatim: "?")
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(hovering || shown ? Ink.paper : Ink.ash)
                .frame(width: 14, height: 14)
                .overlay(Circle().strokeBorder(hovering || shown ? Ink.graphite : Ink.hairline, lineWidth: 1))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .help(Text(text))
        .accessibilityLabel(Text(text))
        .popover(isPresented: $shown, arrowEdge: .bottom) {
            Text(text)
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(Ink.paper)
                .lineSpacing(2)
                .fixedSize(horizontal: false, vertical: true)
                .frame(width: 260, alignment: .leading)
                .padding(Grid.x3)
                .background(Ink.raised)
                .textCase(nil)
                .environment(\.colorScheme, .dark)
        }
    }
}

/// "Allow microphone": asks macOS for the microphone (voice input in the prompt). Shows the
/// answer; when it is "no", offers the system settings.
struct MicrophoneRow: View {
    var nameWidth: CGFloat = 112
    @ObservedObject private var accent = Accent.shared
    @State private var status = AVCaptureDevice.authorizationStatus(for: .audio)

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text("Microphone")
                .foregroundStyle(Ink.graphite)
                .frame(width: nameWidth, alignment: .leading)
            switch status {
            case .authorized:
                HStack(spacing: Grid.x1) {
                    Circle().fill(Ink.accent).frame(width: 5, height: 5)
                    Text("allowed").foregroundStyle(Ink.paper)
                }
                Spacer(minLength: 0)
            case .notDetermined:
                TextButton(title: "Allow microphone", strong: true) {
                    AVCaptureDevice.requestAccess(for: .audio) { _ in
                        DispatchQueue.main.async { status = AVCaptureDevice.authorizationStatus(for: .audio) }
                    }
                }
                Text("for voice input").foregroundStyle(Ink.ash)
                Spacer(minLength: 0)
            default:
                Text("not allowed").foregroundStyle(Ink.ash)
                Spacer(minLength: Grid.x2)
                TextButton(title: "open settings") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        }
        .font(Ink.text(Ink.Size.body))
        .frame(height: 28)
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            status = AVCaptureDevice.authorizationStatus(for: .audio)
        }
    }
}

/// The accent, as a named row of swatches.
struct AccentRow: View {
    var nameWidth: CGFloat = 112

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text("Accent")
                .foregroundStyle(Ink.graphite)
                .frame(width: nameWidth, alignment: .leading)
            AccentPicker()
            Spacer(minLength: 0)
        }
        .font(Ink.text(Ink.Size.body))
        .frame(height: 28)
    }
}

/// A plain text action, the panel's quiet button: graphite, paper and raised on hover;
/// `strong` is outlined.
struct TextButton: View {
    let title: LocalizedStringKey
    var strong = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(hovering || strong ? Ink.paper : Ink.graphite)
                .padding(.horizontal, Grid.x2)
                .frame(height: 24)
                .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(hovering ? Ink.raised : .clear))
                .overlay(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous)
                    .strokeBorder(strong ? Ink.hairline : .clear, lineWidth: 1))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .fixedSize()
    }
}

/// The bottom of the panel: "Settings" folded; open, the accent, the keys and the microphone.
struct PanelSettings: View {
    var stills: StillFrames?
    @AppStorage("settings.open") private var openStored = false
    @State private var hovering = false

    private var open: Bool { stills == nil && openStored }

    var body: some View {
        VStack(alignment: .leading, spacing: Grid.x1) {
            Button { openStored.toggle() } label: {
                HStack(spacing: Grid.x2) {
                    Text("Settings")
                        .font(Ink.text(Ink.Size.body))
                        .foregroundStyle(Ink.paper)
                        .fixedSize()
                    Rectangle().fill(Ink.hairline).frame(height: 1)
                    (open ? Text("close") : Text("accent, keys, microphone"))
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(hovering ? Ink.graphite : Ink.ash)
                        .fixedSize()
                    Text(open ? "⌃" : "⌄")
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(hovering ? Ink.graphite : Ink.ash)
                }
                .frame(height: Popover.headerHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .onHover { hovering = $0 }
            .accessibilityLabel(open ? Text("Close settings") : Text("Open settings"))
            if open {
                VStack(alignment: .leading, spacing: Grid.x1) {
                    AccentRow()
                    KeyRow(account: .typesafe, name: "TypeSafe AI", placeholder: "Add your TypeSafe AI key",
                           help: KeyRow.typesafeHelp)
                    KeyRow(account: .fal, name: "fal.ai", placeholder: "Add your fal key", help: KeyRow.falHelp)
                    MicrophoneRow()
                }
                .padding(.top, Grid.x1)
                .transition(.opacity)
            }
        }
    }
}
