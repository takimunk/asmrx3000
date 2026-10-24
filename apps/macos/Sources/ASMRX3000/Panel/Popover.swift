import CAsmr
import SwiftUI

/// Pre-rendered Metal frames, used instead of live views when writing a snapshot.
struct StillFrames {
    var scene: CGImage
    var spectrogram: CGImage
    var tracks: CGImage
    var spectrogramHover: SIMD2<Float>?
    /// What the snapshot shows (the live popover keeps these in its own state).
    var drawer: UInt32?
    var showAll = false
    var spectrogramOpen = false
    var scenesOpen = false
    /// Sections the snapshot shows open (the user's defaults: world open, sound folded).
    var soundOpen = false
    var worldOpen = true
    var settingsOpen = false
    /// Cap the panel at the screen's room, as the live panel does (default: the whole content).
    var fixedHeight = false
}

/// The popover, top to bottom: the scene (with the scene picker in its corner), one transport
/// row (settings at its end), the prompt, the levers in two sections (world: what happens in the
/// forest, open; sound: loudness per group, folded to a strip of mute buttons) or one group's
/// drawer or the settings, and the spectrogram as a quiet strip. A clip scene (underwater, space)
/// has its own levers and no prompt (it only knows the forest's knobs).
///
/// The panel is as tall as its content, up to the screen's room; only past that do the levers
/// scroll under the stage, the transport and the prompt.
struct Popover: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    @ObservedObject var controls: ControlsModel
    var stills: StillFrames?
    /// What the hovered transport control does, shown over the scene's bottom-right corner.
    @State private var hint: String?
    /// The settings stand in for the levers while open (not remembered: the panel opens on the levers).
    @State private var settingsOpenState = false
    /// Measured heights: the fixed top (stage, transport, prompt) and the levers' content.
    @State private var topHeight: CGFloat = 0
    @State private var contentHeight: CGFloat = 0

    private var settingsOpen: Bool { stills?.settingsOpen ?? settingsOpenState }

    static let width: CGFloat = 540
    static let sceneHeight: CGFloat = 232
    /// The tallest the panel gets: the menu bar screen's usable height, less the 6 pt gap under
    /// the menu bar and a margin above the Dock. Content up to this height shows whole.
    static var maxHeight: CGFloat {
        let room = roomOverride ?? (NSScreen.screens.first?.visibleFrame.height ?? 800)
        return max(420, room - 18)
    }
    /// A pretend screen height (snapshots: `--room`).
    static var roomOverride: CGFloat?
    /// The levers' area never shrinks below this before it scrolls (a small screen).
    static let minScroll: CGFloat = 120
    static let gutter: CGFloat = Grid.x4
    /// Pictograms: 18 pt (1.1 pt line) for buttons and levers, 16 pt (1 pt line) for compact
    /// rows, drawer parameters and chips.
    static let iconSize: CGFloat = 18
    static let smallIconSize: CGFloat = 16
    /// World levers: pictogram, name, slider (the waveform), value, chevron.
    /// A drawer's rows share the levers' height and columns, so names, sliders and values stay
    /// put when a drawer opens.
    static let leverHeight: CGFloat = 26
    static let childHeight: CGFloat = leverHeight
    static let iconColumn: CGFloat = 26
    static let nameColumn: CGFloat = 150
    static let valueColumn: CGFloat = 72
    static let chevronColumn: CGFloat = 20
    /// Sound levels: two columns of compact rows, the same icon column as the levers.
    static let soundRowHeight: CGFloat = 22
    static let soundColumnGap: CGFloat = Grid.x4
    static let soundNameColumn: CGFloat = 76
    static let soundValueColumn: CGFloat = 48
    static var soundColumnWidth: CGFloat { (width - 2 * gutter - soundColumnGap) / 2 }
    static let transportHeight: CGFloat = 48
    /// A section header ("Звук ─── …") and a drawer's title line.
    static let headerHeight: CGFloat = 20
    static let volumeWidth: CGFloat = 48
    static let sceneMenuWidth: CGFloat = 236
    /// The plates over the scene that are buttons (scene picker, settings): one height.
    static let chipHeight: CGFloat = 26
    static let spectrogramClosed: CGFloat = 24
    static let spectrogramOpen: CGFloat = 96
    /// The levers' slider width (the track scopes are drawn to it).
    static var sliderWidth: CGFloat { width - 2 * gutter - iconColumn - nameColumn - valueColumn - chevronColumn }
    static var stripWidth: CGFloat { width - 2 * gutter }

    init(model: AppModel, stills: StillFrames? = nil) {
        self.model = model
        audio = model.audio
        head = model.head
        controls = model.stage.controls
        self.stills = stills
    }

    var body: some View {
        Group {
            if let stills {
                // ImageRenderer draws no ScrollView and lays out once: the whole content, or with
                // --fixed the top of it in the frame the live panel would get.
                VStack(spacing: 0) { top; scrolled }
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxHeight: stills.fixedHeight ? Self.maxHeight : nil, alignment: .top)
                    .clipped()
            } else {
                VStack(spacing: 0) {
                    top.background(Measure { topHeight = $0 })
                    ScrollView(.vertical) { scrolled.background(Measure { contentHeight = $0 }) }
                        .scrollIndicators(.automatic)
                        .frame(height: scrollHeight)
                }
            }
        }
        .frame(width: Self.width)
        .background(Ink.panel)
        .environment(\.colorScheme, .dark)
    }

    /// The levers' area: all of it while the panel fits the screen, else what is left under the top.
    private var scrollHeight: CGFloat {
        guard contentHeight > 0 else { return Self.minScroll }
        return min(contentHeight, max(Self.minScroll, Self.maxHeight - topHeight))
    }

    /// What never scrolls: the stage, the transport and, in the forest, the prompt.
    private var top: some View {
        VStack(spacing: 0) {
            SceneArea(model: model, stills: stills, hint: $hint, settingsOpen: settingsOpen) {
                if Motion.reduced { settingsOpenState.toggle() } else { withAnimation(.easeOut(duration: 0.16)) { settingsOpenState.toggle() } }
                model.stage.focusScene?()
            }
                .arrival(0)
            Transport(model: model, audio: audio, head: head, labels: model.stage.labels, meters: model.stage.meters,
                      controls: controls, hint: $hint)
                .arrival(1)
            if !audio.scene.isClip {
                PromptField(prompt: model.prompt, stage: model.stage)
                    .padding(.horizontal, Self.gutter)
                    .padding(.top, Grid.x1)
                    .arrival(2)
            }
        }
    }

    /// Everything under the prompt: the levers, a drawer or the settings, then the spectrogram.
    private var scrolled: some View {
        VStack(spacing: 0) {
            Group {
                if settingsOpen {
                    SettingsDrawer { settingsOpenState = false }
                } else {
                    LeverPanel(model: model, controls: controls, stills: stills)
                }
            }
            .padding(.horizontal, Self.gutter)
            .padding(.top, audio.scene.isClip ? Grid.x1 : Grid.x3)
            .arrival(3)
            SpectrogramStrip(stage: model.stage, stills: stills)
                .padding(.horizontal, Self.gutter)
                .padding(.top, Grid.x3)
                .padding(.bottom, Grid.x4)
                .arrival(4)
        }
        .frame(width: Self.width)
    }
}

/// Reports the height of the view it is the background of.
private struct Measure: View {
    let changed: (CGFloat) -> Void

    var body: some View {
        GeometryReader { geo in
            Color.clear
                .onAppear { changed(geo.size.height) }
                .onChange(of: geo.size.height) { _, h in changed(h) }
        }
    }
}

// MARK: - Scene

private struct SceneArea: View {
    let model: AppModel
    var stills: StillFrames?
    @Binding var hint: String?
    let settingsOpen: Bool
    let toggleSettings: () -> Void
    @ObservedObject private var lang = Language.shared
    @State private var hovering = false

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            Group {
                if let stills {
                    Image(decorative: stills.scene, scale: 2).resizable()
                } else {
                    SceneSurface(stage: model.stage)
                }
            }
            .frame(width: Popover.width, height: Popover.sceneHeight)
            .accessibilityLabel(tr("Scene"))

            SelectionChip(readout: model.stage.readout, meters: model.stage.meters, showHint: hovering || stills != nil,
                          clipScene: model.audio.scene.isClip)
                .padding(Grid.x3)
                // The transport's hint takes the corner while it is up, so the two never overlap.
                .opacity(hint == nil ? 1 : 0)
        }
        .overlay(alignment: .bottomTrailing) {
            if let hint {
                Text(hint)
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.paper)
                    .lineLimit(1)
                    .chip()
                    .padding(Grid.x3)
                    .allowsHitTesting(false)
                    .transition(.opacity)
            }
        }
        .animation(Motion.reduced ? nil : .easeOut(duration: 0.12), value: hint)
        .overlay(alignment: .topLeading) {
            ScenePicker(model: model, audio: model.audio, forceOpen: stills?.scenesOpen ?? false)
                .padding(Grid.x3)
        }
        .overlay(alignment: .topTrailing) {
            HStack(spacing: Grid.x2) {
                SessionChip(audio: model.audio, hint: $hint)
                SettingsChip(open: settingsOpen, hint: $hint, toggle: toggleSettings)
            }
            .padding(Grid.x3)
        }
        .frame(width: Popover.width, height: Popover.sceneHeight)
        .clipped()
        .onHover { hovering = $0 }
    }
}

/// A timed session (a scene with a script, such as the body scan): time gone and left, and a
/// button that starts it over. Polled once a second; absent in scenes without a script.
private struct SessionChip: View {
    let audio: AudioEngine
    @Binding var hint: String?
    @ObservedObject private var lang = Language.shared
    @State private var hovering = false

    static func clock(_ s: Float) -> String {
        let t = Swift.max(0, Int(s.rounded()))
        return String(format: "%d:%02d", t / 60, t % 60)
    }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            let d = audio.directionInfo()
            if d.script_s > 0 {
                let done = d.elapsed_s >= d.script_s
                let name = tr("Start over")
                HStack(spacing: Grid.x2) {
                    Text(done ? tr("finished") : Self.clock(d.elapsed_s))
                        .foregroundStyle(Ink.paper)
                    if !done {
                        Text(tr("%@ left", Self.clock(d.script_s - d.elapsed_s)))
                            .foregroundStyle(Ink.ash)
                    }
                    Button { audio.restartScene() } label: {
                        Text("↺")
                            .foregroundStyle(hovering ? Ink.paper : Ink.paper.opacity(0.8))
                            .frame(width: Grid.x4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .focusable(false)
                    .onHover { h in
                        hovering = h
                        hint = h ? name : nil
                    }
                    .help(name)
                    .accessibilityLabel(name)
                }
                .font(Ink.text(Ink.Size.caption).monospacedDigit())
                .lineLimit(1)
                .padding(.horizontal, Grid.x3)
                .frame(height: Popover.chipHeight)
                .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(Ink.panel.opacity(0.88)))
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// The settings, in the scene's top-right corner opposite the scene picker: a plate of the same
/// height with the rails pictogram; raised while the settings stand in for the levers. ⌘,
private struct SettingsChip: View {
    let open: Bool
    @Binding var hint: String?
    let toggle: () -> Void
    @ObservedObject private var lang = Language.shared
    @State private var hovering = false

    var body: some View {
        let name = open ? tr("Close settings") : tr("Settings: language, accent, keys, microphone")
        Button(action: toggle) {
            IconView(icon: .settings, color: hovering || open ? Ink.paper : Ink.paper.opacity(0.85), size: Popover.smallIconSize,
                     animating: hovering, lineWidth: 1)
                .frame(width: Popover.chipHeight + Grid.x1, height: Popover.chipHeight)
                .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous)
                    .fill(open ? Ink.raised.opacity(0.94) : hovering ? Ink.raised.opacity(0.9) : Ink.panel.opacity(0.88)))
                .overlay(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous)
                    .strokeBorder(open ? Ink.hairline : .clear, lineWidth: 1))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .keyboardShortcut(",", modifiers: .command)
        .onHover { h in
            hovering = h
            hint = h ? "\(name)   ⌘," : nil
        }
        .help(name)
        .accessibilityLabel(name)
        .accessibilityAddTraits(open ? .isSelected : [])
    }
}

/// The selected thing, where it is and how to move it; with nothing selected, how to select.
private struct SelectionChip: View {
    @ObservedObject var readout: SceneReadout
    @ObservedObject var meters: Meters
    let showHint: Bool
    var clipScene = false
    @ObservedObject private var lang = Language.shared

    var body: some View {
        Group {
            if let s = readout.selection {
                VStack(alignment: .leading, spacing: Grid.x1) {
                    HStack(spacing: Grid.x2) {
                        IconView(icon: icon(s.kind), color: Ink.paper, size: Popover.smallIconSize, lineWidth: 1)
                        Text(s.title.map(ControlText.sentence) ?? name(s.kind)).font(Ink.text(Ink.Size.title)).foregroundStyle(Ink.paper)
                        Text(coordinates(s.position, withHeight: s.kind == .bird || s.kind == .clip))
                            .font(Ink.text(Ink.Size.caption))
                            .foregroundStyle(Ink.graphite)
                    }
                    Text(s.kind == .clip ? tr("%@   tab next   esc release", s.detail ?? tr("Moves on its own"))
                         : !s.movable ? tr("In flight: you can move it once it lands")
                         : s.kind == .bird ? tr("← → ↑ ↓ move   ⇧↑ ⇧↓ height   esc release")
                         : tr("← → ↑ ↓ move   esc release"))
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.graphite)
                }
                .chip()
            } else if showHint && clipScene {
                Text(tr("Click a sound to see what it is and where"))
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.graphite)
                    .chip()
            } else if showHint {
                Text(tr("Click a tree, a bird or the river to move it"))
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.graphite)
                    .chip()
            }
        }
        .allowsHitTesting(false)
        .animation(Motion.reduced ? nil : .easeOut(duration: 0.15), value: readout.selection?.kind)
    }

    private func name(_ kind: Thing.Kind) -> String {
        switch kind {
        case .tree: return tr("Tree")
        case .bird: return tr("Bird")
        case .river: return tr("River")
        case .clip: return tr("Sound")
        }
    }

    private func icon(_ kind: Thing.Kind) -> Icon {
        switch kind {
        case .tree: return .tree
        case .bird: return .bird
        case .river: return .river
        case .clip: return .tone
        }
    }

    /// Relative to the listener. Trees and the river stand on the ground, so only birds and
    /// clips get a height.
    private func coordinates(_ p: SIMD3<Float>, withHeight: Bool) -> String {
        // + 0 turns −0 into 0, so a sound level with the ears does not read "−0.0".
        func f(_ v: Float) -> String { L10n.number(Double((v * 10).rounded() / 10 + 0), digits: 1) }
        let ahead = p.x >= 0 ? tr("%@ m ahead", f(p.x)) : tr("%@ m behind", f(-p.x))
        let side = p.y >= 0 ? tr("%@ m left", f(p.y)) : tr("%@ m right", f(-p.y))
        let height = p.z >= 0 ? tr("%@ m up", f(p.z)) : tr("%@ m down", f(-p.z))
        return withHeight ? "\(ahead), \(side), \(height)" : "\(ahead), \(side)"
    }
}

/// The scene in the scene's top-left corner; click it for the list. Switching builds the new
/// scene in the background (clip files take a moment), then the sound carries on in it.
private struct ScenePicker: View {
    let model: AppModel
    @ObservedObject var audio: AudioEngine
    var forceOpen = false
    @ObservedObject private var lang = Language.shared
    @State private var openState = false
    @State private var hovered: String?
    @State private var hoveringTitle = false

    private var open: Bool { forceOpen || openState }

    var body: some View {
        ZStack(alignment: .topLeading) {
            if openState {
                // A click anywhere else on the scene closes the list.
                Color.clear
                    .contentShape(Rectangle())
                    .frame(width: Popover.width, height: Popover.sceneHeight)
                    .offset(x: -Grid.x3, y: -Grid.x3)
                    .onTapGesture { openState = false }
            }
            VStack(alignment: .leading, spacing: Grid.x1) {
                Button { openState.toggle() } label: {
                    HStack(spacing: Grid.x2) {
                        Text(audio.scene.title)
                            .font(Ink.text(Ink.Size.title))
                            .foregroundStyle(hoveringTitle || open ? Ink.paper : Ink.paper.opacity(0.85))
                        if let loading = audio.loadingScene {
                            Text("→ " + loading.title.lowercased() + "…")
                                .font(Ink.text(Ink.Size.caption))
                                .foregroundStyle(Ink.graphite)
                        }
                        IconView(icon: .chevron, color: hoveringTitle || open ? Ink.paper : Ink.graphite, size: 12, lineWidth: 1)
                            .rotationEffect(.degrees(open ? -90 : 90))
                    }
                    .chip()
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .onHover { hoveringTitle = $0 }
                .help(tr("Scene"))
                .accessibilityLabel(tr("Scene: %@", audio.scene.title))

                if open {
                    Group {
                        if forceOpen {
                            list
                        } else {
                            ScrollView(.vertical) { list }.scrollIndicators(.never)
                        }
                    }
                    .frame(height: min(CGFloat(SceneEntry.all.count) * (Grid.x6 + Grid.x1),
                                       Popover.sceneHeight - 2 * Grid.x3 - 40))
                    .clipped()
                    .padding(.vertical, Grid.x1)
                    .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(Ink.panel.opacity(0.94)))
                    .overlay(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).strokeBorder(Ink.hairline, lineWidth: 1))
                }
                if let error = audio.sceneError, !open {
                    Text(error)
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.graphite)
                        .chip()
                }
            }
        }
        .animation(Motion.reduced ? nil : .easeOut(duration: 0.12), value: open)
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(SceneEntry.all) { e in row(e) }
        }
    }

    private func row(_ e: SceneEntry) -> some View {
        let current = e == audio.scene.entry
        let loading = e == audio.loadingScene
        let lit = hovered == e.id || current
        return Button {
            openState = false
            audio.switchScene(to: e)
            model.stage.focusScene?()
        } label: {
            HStack(spacing: Grid.x1) {
                Text(current ? "›" : loading ? "…" : " ")
                    .foregroundStyle(Ink.paper)
                    .frame(width: Grid.x2, alignment: .leading)
                Text(e.title)
                    .foregroundStyle(lit ? Ink.paper : Ink.graphite)
                Spacer(minLength: Grid.x4)
                Text(Self.kind(e))
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(hovered == e.id ? Ink.graphite : Ink.ash)
            }
            .font(Ink.text(Ink.Size.body))
            .padding(.horizontal, Grid.x3)
            .frame(width: Popover.sceneMenuWidth, height: Grid.x6 + Grid.x1)
            .background(RoundedRectangle(cornerRadius: Grid.innerRadius, style: .continuous)
                .fill(hovered == e.id ? Ink.raised : .clear).padding(.horizontal, Grid.x1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { h in hovered = h ? e.id : (hovered == e.id ? nil : hovered) }
        .accessibilityLabel(e.title)
        .accessibilityAddTraits(current ? .isSelected : [])
    }

    /// What kind of scene it is, quietly on the right.
    private static func kind(_ e: SceneEntry) -> String {
        if e.id.hasSuffix("_real") { return tr("recordings") }
        if e.id.hasPrefix("underwater") || e.id.hasPrefix("space") { return tr("sounds") }
        return tr("model")
    }
}

extension View {
    /// Keyboard focus: one ring for every focusable control, concentric with its shape.
    func focusRing(_ on: Bool, radius: CGFloat) -> some View {
        overlay(RoundedRectangle(cornerRadius: radius + Grid.focusGap, style: .continuous)
            .strokeBorder(on ? Ink.paper.opacity(0.7) : .clear, lineWidth: Grid.focusRing)
            .padding(-Grid.focusGap))
    }
}

private extension View {
    /// A small dark plate that keeps text legible over the scene.
    func chip() -> some View {
        padding(.horizontal, Grid.x3)
            .padding(.vertical, Grid.x2 - 1)
            .background(RoundedRectangle(cornerRadius: Grid.chipRadius, style: .continuous).fill(Ink.panel.opacity(0.88)))
    }
}

// MARK: - Transport

/// Play, volume, where the listener is (home, standing in the world or flying), head tracking and recenter;
/// the status on the right says where the head looks (or the hour). Whatever the pointer is
/// over is named, with its key, in `hint` (drawn over the scene).
private struct Transport: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    @ObservedObject var labels: SceneLabels
    @ObservedObject var meters: Meters
    @ObservedObject var controls: ControlsModel
    @Binding var hint: String?
    @ObservedObject private var lang = Language.shared

    var body: some View {
        HStack(spacing: Grid.x1) {
            ToolButton(icon: audio.isPlaying ? .pause : .play, name: audio.isPlaying ? tr("Pause") : tr("Play"), key: tr("space"),
                       style: audio.isPlaying ? .plain : .primary, hint: $hint, action: act { model.togglePlay() })
            if let volume = controls.macros.first(where: { Int($0.group) == ASMR_GROUP_MASTER }) {
                VolumeSlider(control: volume, hint: $hint) { v in
                    model.stage.poke()
                    audio.setControl(volume.id, v)
                    model.stage.refreshControls()
                }
                .padding(.trailing, Grid.x2)
            }
            if !audio.scene.isClip {
                // Home, standing or flying: the forest's generated world. Clip scenes have none.
                ModeSwitch(place: audio.place, hint: $hint) { to in
                    act { if audio.place != to { audio.place = to } }()
                }
                .padding(.trailing, Grid.x1)
            }
            LabeledToggle(icon: .head, title: tr("Head"), on: head.enabled,
                          name: head.enabled ? tr("Head tracking is on: turn off") : tr("Head tracking with AirPods: turn on"),
                          key: "H", hint: $hint, action: act { head.enabled.toggle() })
                .contextMenu { rerunOnboarding }
            ToolButton(icon: .recenter, name: tr("Face forward from here"), key: "C", style: .plain, hint: $hint, action: act { head.recenter() })
                .disabled(!head.enabled)
                .contextMenu { rerunOnboarding }
            Spacer(minLength: 0)
            status
            ToolButton(icon: .power, name: tr("Quit ASMR X3000"), key: "⌘Q", style: .quiet, hint: $hint) { NSApp.terminate(nil) }
                .keyboardShortcut("q", modifiers: .command)
        }
        // Button frames sit on the gutter, flush with the prompt's frame below.
        .padding(.horizontal, Popover.gutter)
        .frame(height: Popover.transportHeight)
    }

    /// Right click on "Голова" or recenter: the first-launch walkthrough again (permission,
    /// the head that follows yours, "look at the centre", the screen's size and distance).
    private var rerunOnboarding: some View {
        Button(tr("Set up head and screen again…")) { model.onboarding.show() }
    }

    /// Runs a button's action and gives the keyboard back to the scene, so arrows keep moving things.
    private func act(_ f: @escaping () -> Void) -> () -> Void {
        { f(); model.stage.focusScene?() }
    }

    /// Where the head is turned (the violet dot on "Голова" already says tracking is on), or
    /// the hour when it is off.
    private var status: some View {
        Group {
            if head.enabled {
                Text(labels.gaze.map(Self.shortGaze) ?? (head.connected ? "AirPods" : tr("no AirPods")))
                    .foregroundStyle(labels.gaze == nil ? Ink.ash : Ink.graphite)
            } else if !audio.scene.isClip {
                // The hour is also the world's "Время суток" lever; here only when there is room.
                Text(labels.time).foregroundStyle(Ink.ash)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(1)
        .truncationMode(.tail)
        .padding(.trailing, Grid.x1)
        .accessibilityHidden(true)
    }

    /// The scene's gaze label ("35° left, 15° up", Stage.swift), short and language-free:
    /// "← 35° ↑ 15°"; "straight ahead" is localized.
    static func shortGaze(_ gaze: String) -> String {
        let arrows = ["left": "←", "right": "→", "up": "↑", "down": "↓", "straight ahead": tr("straight ahead")]
        return gaze.components(separatedBy: ", ").map { part in
            let words = part.split(separator: " ", maxSplits: 1).map(String.init)
            guard words.count == 2, let arrow = arrows[words[1]] else { return arrows[part] ?? part }
            return "\(arrow) \(words[0])"
        }.joined(separator: " ")
    }
}

/// Where the listener is: "Дома | Стоим | Летим", the current one raised. Clicking another
/// one switches; F stops / flies on in the world and takes off from home. Says plainly which
/// mode is on and what a click does.
private struct ModeSwitch: View {
    typealias Place = AudioEngine.Place
    let place: Place
    @Binding var hint: String?
    let set: (Place) -> Void
    @ObservedObject private var lang = Language.shared
    @State private var hovered: Place?
    /// The segments sit 2 pt inside the frame.
    private static let inset: CGFloat = 2

    var body: some View {
        HStack(spacing: Self.inset) {
            segment(.home, title: tr("Home"))
            segment(.standing, title: tr("Standing"))
            segment(.flying, title: tr("Flying"))
        }
        .padding(Self.inset)
        .frame(height: Grid.control)
        .background(RoundedRectangle(cornerRadius: Grid.radius, style: .continuous).strokeBorder(Ink.hairline, lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(tr("Mode"))
    }

    /// What hovering a segment says: the current mode, or what a click does (and its key).
    private func describe(_ to: Place) -> String {
        if to == place {
            switch to {
            case .home: return tr("Now: at home, the forest by the river")
            case .standing: return tr("Now: standing, walk   W A S D,  run   ⇧")
            case .flying: return tr("Now: flying, steer   A D,  faster   W")
            }
        }
        switch to {
        case .home: return tr("Go back home")
        case .standing: return place == .flying ? tr("Stop here   F") : tr("Step into the world and stand")
        case .flying: return place == .standing ? tr("Fly on   F") : tr("Take off over the forest   F")
        }
    }

    private func segment(_ to: Place, title: String) -> some View {
        let on = place == to
        return Button { set(to) } label: {
            Text(title)
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(on || hovered == to ? Ink.paper : Ink.graphite)
                .fixedSize()
                .padding(.horizontal, Grid.x2 - 2)
                .frame(maxHeight: .infinity)
                .background(RoundedRectangle(cornerRadius: Grid.radius - Self.inset, style: .continuous)
                    .fill(on ? Ink.pressed : hovered == to ? Ink.raised : .clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { h in
            hovered = h ? to : (hovered == to ? nil : hovered)
            hint = h ? describe(to) : nil
        }
        .accessibilityLabel(title)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// A toggle with its name written next to the pictogram: raised and bright when on, the
/// pictogram struck through when off.
private struct LabeledToggle: View {
    let icon: Icon
    let title: String
    let on: Bool
    let name: String
    var key: String?
    @Binding var hint: String?
    let action: () -> Void
    @State private var hovering = false
    @FocusState private var focused: Bool

    var body: some View {
        Button(action: action) {
            HStack(spacing: Grid.x1 + 2) {
                IconView(icon: icon, color: on || hovering ? Ink.paper : Ink.graphite, size: Popover.iconSize,
                         animating: hovering && on, struck: !on)
                Text(title)
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(on || hovering ? Ink.paper : Ink.graphite)
                    .fixedSize()
                if on {
                    Circle().fill(Ink.violet).frame(width: 5, height: 5)
                }
            }
            .padding(.horizontal, Grid.x2)
            .frame(height: Grid.control)
            .contentShape(Rectangle())
        }
        .buttonStyle(ToolStyle(style: on ? .on : .plain, hovering: hovering, focused: focused))
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onHover { h in
            hovering = h
            hint = h ? (key.map { "\(name)   \($0)" } ?? name) : nil
        }
        .help(name)
        .accessibilityLabel(name)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// A square button (Grid.control): visible hit area on hover, pressed and focus states; `.primary` is
/// filled (the one thing to do when paused), `.on` marks a toggle that is on.
private struct ToolButton: View {
    enum Style { case plain, primary, on, quiet }
    let icon: Icon
    let name: String
    var key: String?
    var style: Style = .plain
    var live = false
    @Binding var hint: String?
    let action: () -> Void
    @State private var hovering = false
    @FocusState private var focused: Bool
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: action) {
            IconView(icon: icon, color: iconColor, size: Popover.iconSize, animating: hovering && live)
                .frame(width: Grid.control, height: Grid.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(ToolStyle(style: style, hovering: hovering && enabled, focused: focused))
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onHover { h in
            hovering = h
            hint = h ? (key.map { "\(name)   \($0)" } ?? name) : nil
        }
        .help(name)
        .accessibilityLabel(name)
    }

    private var iconColor: Color {
        if !enabled { return Ink.ash }
        switch style {
        case .primary: return Ink.panel
        case .on: return Ink.paper
        case .quiet: return hovering ? Ink.paper : Ink.ash
        case .plain: return hovering ? Ink.paper : Ink.graphite
        }
    }
}

private struct ToolStyle: ButtonStyle {
    let style: ToolButton.Style
    let hovering: Bool
    let focused: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(RoundedRectangle(cornerRadius: Grid.radius, style: .continuous).fill(fill(configuration.isPressed)))
            .overlay(RoundedRectangle(cornerRadius: Grid.radius, style: .continuous)
                .strokeBorder(style == .on ? Ink.hairline : .clear, lineWidth: 1))
            .focusRing(focused, radius: Grid.radius)
    }

    private func fill(_ pressed: Bool) -> Color {
        switch style {
        case .primary: return pressed ? Ink.graphite : hovering ? .white : Ink.paper
        case .on: return pressed ? Ink.pressed : hovering ? Ink.pressed : Ink.raised
        case .plain, .quiet: return pressed ? Ink.pressed : hovering ? Ink.raised : .clear
        }
    }
}

/// The master level, small, next to play (the one loudness that is not in the sound section).
private struct VolumeSlider: View {
    let control: Control
    @Binding var hint: String?
    let set: (Float) -> Void
    @ObservedObject private var lang = Language.shared
    @State private var hovering = false
    @State private var dragging = false

    var body: some View {
        HStack(spacing: Grid.x1) {
            IconView(icon: .gain, color: hovering || dragging ? Ink.paper : Ink.graphite, size: Popover.smallIconSize, lineWidth: 1)
            GeometryReader { geo in
                let w = geo.size.width, x = CGFloat(control.fraction) * w
                let knob = hovering || dragging ? Grid.knobActive : Grid.knob
                ZStack(alignment: .leading) {
                    Capsule().fill(Ink.hairline).frame(height: Grid.rail)
                    Capsule().fill(Ink.graphite).frame(width: x, height: Grid.rail)
                    RoundedRectangle(cornerRadius: 1)
                        .fill(Ink.paper)
                        .frame(width: knob.width, height: knob.height)
                        .offset(x: min(max(x - knob.width / 2, 0), w - knob.width))
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { g in
                        dragging = true
                        set(control.value(atFraction: Float(g.location.x / w)))
                        hint = tr("Volume") + "   \(Int((control.fraction * 100).rounded())) %"
                    }
                    .onEnded { _ in dragging = false; if !hovering { hint = nil } })
            }
            .frame(width: Popover.volumeWidth, height: Grid.control)
        }
        .onHover { h in
            hovering = h
            hint = h ? tr("Volume") + "   \(Int((control.fraction * 100).rounded())) %" : (dragging ? hint : nil)
        }
        .accessibilityElement()
        .accessibilityLabel(tr("Volume"))
        .accessibilityValue("\(Int(control.fraction * 100)) %")
        .accessibilityAdjustableAction { dir in
            set(control.value(atFraction: control.fraction + (dir == .increment ? 0.05 : -0.05)))
        }
    }
}

// MARK: - Spectrogram

/// A thin live strip by default; click it to open the full spectrogram with its scale.
private struct SpectrogramStrip: View {
    let stage: Stage
    var stills: StillFrames?
    @AppStorage("spectrogram.open") private var openStored = false
    @ObservedObject private var lang = Language.shared
    @State private var hover: SIMD2<Float>?
    @State private var zoom = SIMD2<Float>(0, 1)
    @State private var frozen = false
    @State private var hovering = false

    private var open: Bool { stills.map(\.spectrogramOpen) ?? openStored }

    var body: some View {
        let h = open ? Popover.spectrogramOpen : Popover.spectrogramClosed
        let w = Popover.stripWidth
        ZStack(alignment: .topLeading) {
            Group {
                if let stills {
                    Image(decorative: stills.spectrogram, scale: 2).resizable()
                } else {
                    SpectrogramSurface(stage: stage) { p in
                        hover = p
                        zoom = stage.zoom
                        frozen = stage.frozen
                    }
                }
            }
            .frame(width: w, height: h)
            .clipShape(RoundedRectangle(cornerRadius: Grid.innerRadius, style: .continuous))
            .accessibilityLabel(tr("Spectrogram"))

            if open {
                ForEach([100, 1000, 10_000], id: \.self) { hz in
                    let y = yFor(Float(hz), h: h)
                    if y > 7 && y < h - 7 {
                        Text(hz >= 1000 ? tr("%d kHz", hz / 1000) : tr("%d Hz", hz))
                            .font(Ink.text(Ink.Size.caption))
                            .foregroundStyle(Ink.ash)
                            .fixedSize()
                            .padding(.horizontal, Grid.x1)
                            .background(Ink.panel.opacity(0.7))
                            .frame(width: w, alignment: .leading)
                            .padding(.leading, Grid.x2)
                            .position(x: w / 2 + Grid.x2 / 2, y: CGFloat(y))
                            .allowsHitTesting(false)
                    }
                }
                if let p = hover ?? stills?.spectrogramHover {
                    Text(readout(at: p, w: Float(w), h: Float(h)))
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.paper)
                        .padding(.horizontal, Grid.x2)
                        .padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: Grid.innerRadius, style: .continuous).fill(Ink.panel.opacity(0.85)))
                        .frame(maxWidth: .infinity, alignment: .trailing)
                        .padding(.top, Grid.x1)
                        .padding(.trailing, Grid.x6 + Grid.x2)
                        .allowsHitTesting(false)
                }
            } else {
                // Closed, the whole strip is one button.
                Button { openStored = true } label: { Color.clear.contentShape(Rectangle()) }
                    .buttonStyle(.plain)
                    .frame(width: w, height: h)
                    .accessibilityLabel(tr("Open the spectrogram"))
            }

            Button { openStored.toggle() } label: {
                IconView(icon: .chevron, color: hovering ? Ink.paper : Ink.graphite, size: 14, lineWidth: 1)
                    .rotationEffect(.degrees(open ? -90 : 90))
                    .frame(width: Grid.x6, height: Grid.x6)
                    .background(RoundedRectangle(cornerRadius: Grid.innerRadius, style: .continuous).fill(Ink.panel.opacity(0.7)))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .padding(.top, open ? Grid.x1 : 0)
            .padding(.trailing, open ? Grid.x1 : 0)
            .accessibilityLabel(open ? tr("Close the spectrogram") : tr("Open the spectrogram"))
        }
        .frame(width: w, height: h)
        .onHover { hovering = $0 }
        .help(open ? tr("Click: freeze, wheel: zoom, double click: reset") : tr("Spectrogram"))
        .onAppear { zoom = stage.zoom }
    }

    private func yFor(_ hz: Float, h: CGFloat) -> CGFloat {
        let f = stage.fraction(forFrequency: hz)
        return CGFloat(1 - (f - zoom.x) / (zoom.y - zoom.x)) * h
    }

    private func readout(at p: SIMD2<Float>, w: Float, h: Float) -> String {
        let z = stills == nil ? zoom : stage.zoom
        let frac = z.x + (z.y - z.x) * (1 - p.y / h)
        let hz = stage.frequency(atFraction: frac)
        let back = Int((w - p.x) / w * Float(SpectrogramRenderer.visibleColumns))
        let v = stage.historyValue(column: back, band: frac)
        let db = Int((v * 70 - 90).rounded())
        let freq = hz >= 1000 ? tr("%.2f kHz", Double(hz / 1000)) : tr("%d Hz", Int(hz.rounded()))
        let level = tr("%d dB", db).replacingOccurrences(of: "-", with: "−")
        return "\(freq)  \(level)\(frozen ? "  " + tr("frozen") : "")"
    }
}
