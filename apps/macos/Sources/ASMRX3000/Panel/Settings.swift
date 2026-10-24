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
    let name: String
    /// The English key of the placeholder, shown through `tr`.
    let placeholder: String
    var nameWidth: CGFloat = 112
    /// Why the key is needed (an English key): a "?" after the name, tooltip on hover, popover on click.
    var help: String?
    @ObservedObject private var accent = Accent.shared
    @ObservedObject private var lang = Language.shared
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
                    Text(tr(name)).foregroundStyle(Ink.graphite)
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
            Text(tr("set by %@", account.env)).foregroundStyle(Ink.ash)
            Spacer(minLength: 0)
        } else {
            HStack(spacing: Grid.x1) {
                Circle().fill(Ink.accent).frame(width: 5, height: 5)
                Text(justSaved ? tr("saved") : tr("added")).foregroundStyle(Ink.paper)
            }
            Spacer(minLength: Grid.x2)
            TextButton(title: tr("change")) { editing = true; focused = true }
            TextButton(title: tr("remove")) {
                Keychain.remove(account)
                stored = Keychain.has(account)
                justSaved = false
            }
        }
    }

    private var field: some View {
        HStack(spacing: Grid.x1) {
            SecureField("", text: $value, prompt: Text(tr(placeholder)).foregroundColor(Ink.ash))
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
                TextButton(title: tr("save"), strong: true, action: save)
            } else if editing {
                TextButton(title: tr("cancel")) { editing = false; problem = nil }
            }
        }
    }

    /// What each key is for (Prompt/TypeSafeProvider.swift, Prompt/VoiceInput.swift).
    static let typesafeHelp: String = N_("TypeSafe AI powers the text prompt. What you type (\"quieter river, night\") is sent with the scene's current state, and it picks which sound and world controls to change and how fast. It also answers questions about what is around you. Without this key the prompt does nothing.")
    static let falHelp: String = N_("fal.ai powers voice input. Click or hold the microphone in the prompt and speak (in your language, up to 30 s); the recording goes to fal.ai's Whisper, which turns it into text, and the text is sent as a prompt, so voice also needs the TypeSafe AI key. Audio leaves the Mac only while you record.")

    private func save() {
        if let p = Keychain.problem(value, account: account) {
            problem = p
            return
        }
        guard Keychain.save(value, account: account) else {
            problem = tr("could not save to the keychain")
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
    let text: String
    @ObservedObject private var lang = Language.shared
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
        .help(tr(text))
        .accessibilityLabel(tr(text))
        .popover(isPresented: $shown, arrowEdge: .bottom) {
            Text(tr(text))
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
    @ObservedObject private var lang = Language.shared
    @State private var status = AVCaptureDevice.authorizationStatus(for: .audio)

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text(tr("Microphone"))
                .foregroundStyle(Ink.graphite)
                .frame(width: nameWidth, alignment: .leading)
            switch status {
            case .authorized:
                HStack(spacing: Grid.x1) {
                    Circle().fill(Ink.accent).frame(width: 5, height: 5)
                    Text(tr("allowed")).foregroundStyle(Ink.paper)
                }
                Spacer(minLength: 0)
            case .notDetermined:
                TextButton(title: tr("Allow microphone"), strong: true) {
                    AVCaptureDevice.requestAccess(for: .audio) { _ in
                        DispatchQueue.main.async { status = AVCaptureDevice.authorizationStatus(for: .audio) }
                    }
                }
                Text(tr("for voice input")).foregroundStyle(Ink.ash)
                Spacer(minLength: 0)
            default:
                Text(tr("not allowed")).foregroundStyle(Ink.ash)
                Spacer(minLength: Grid.x2)
                TextButton(title: tr("open settings")) {
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
    @ObservedObject private var lang = Language.shared

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text(tr("Accent"))
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
    let title: String
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

/// The settings in the levers' place, opened from the transport's settings button (⌘,): a
/// drawer like a group's, "‹ Settings ───── language, accent, keys, microphone", then the rows.
struct SettingsDrawer: View {
    let close: () -> Void
    @ObservedObject private var lang = Language.shared
    @State private var hoveringBack = false

    var body: some View {
        VStack(alignment: .leading, spacing: Grid.x1) {
            HStack(spacing: Grid.x2) {
                Button(action: close) {
                    HStack(spacing: Grid.x1) {
                        IconView(icon: .back, color: hoveringBack ? Ink.paper : Ink.graphite, size: 14, lineWidth: 1)
                        Text(tr("Settings"))
                            .font(Ink.text(Ink.Size.body))
                            .foregroundStyle(Ink.paper)
                            .fixedSize()
                    }
                    .padding(.leading, Grid.x1)
                    .padding(.trailing, Grid.x2)
                    .frame(height: Popover.headerHeight)
                    .background(RoundedRectangle(cornerRadius: Grid.innerRadius, style: .continuous)
                        .fill(hoveringBack ? Ink.raised : .clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .padding(.leading, -Grid.x1)
                .onHover { hoveringBack = $0 }
                .keyboardShortcut(.leftArrow, modifiers: .command)
                .help(tr("Back   ⌘←"))
                .accessibilityLabel(tr("Close settings"))
                Rectangle().fill(Ink.hairline).frame(height: 1)
                Text(tr("language, accent, keys, microphone"))
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.ash)
                    .lineLimit(1)
                    .fixedSize()
            }
            .frame(height: Popover.headerHeight)
            VStack(alignment: .leading, spacing: Grid.x1) {
                LanguageRow()
                AccentRow()
                KeyRow(account: .typesafe, name: "TypeSafe AI", placeholder: N_("Add your TypeSafe AI key"),
                       help: KeyRow.typesafeHelp)
                KeyRow(account: .fal, name: "fal.ai", placeholder: N_("Add your fal key"), help: KeyRow.falHelp)
                MicrophoneRow()
            }
        }
    }
}
