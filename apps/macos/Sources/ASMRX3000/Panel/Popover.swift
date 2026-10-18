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
}

/// The popover, top to bottom: the scene (most of it), one transport row, the prompt, a few
/// macro levers (or one group's drawer), and the spectrogram as a quiet strip.
struct Popover: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    @ObservedObject var controls: ControlsModel
    var stills: StillFrames?

    static let width: CGFloat = 540
    static let sceneHeight: CGFloat = 384
    static let gutter: CGFloat = Grid.x4
    static let leverHeight: CGFloat = 28
    static let childHeight: CGFloat = 28
    static let iconColumn: CGFloat = 36
    static let chevronColumn: CGFloat = 28
    static let spectrogramClosed: CGFloat = 24
    static let spectrogramOpen: CGFloat = 96
    /// The levers' slider width (the track scopes are drawn to it).
    static var sliderWidth: CGFloat { width - 2 * gutter - iconColumn - chevronColumn }
    static var stripWidth: CGFloat { width - 2 * gutter }

    init(model: AppModel, stills: StillFrames? = nil) {
        self.model = model
        audio = model.audio
        head = model.head
        controls = model.stage.controls
        self.stills = stills
    }

    var body: some View {
        VStack(spacing: 0) {
            SceneArea(model: model, stills: stills)
            Transport(model: model, audio: audio, head: head, labels: model.stage.labels, meters: model.stage.meters,
                      controls: controls)
            PromptField(prompt: model.prompt, stage: model.stage)
                .padding(.horizontal, Self.gutter)
                .padding(.top, Grid.x1)
            LeverPanel(model: model, controls: controls, stills: stills)
                .padding(.horizontal, Self.gutter)
                .padding(.top, Grid.x3)
            SpectrogramStrip(stage: model.stage, stills: stills)
                .padding(.horizontal, Self.gutter)
                .padding(.top, Grid.x3)
                .padding(.bottom, Grid.x4)
        }
        .frame(width: Self.width)
        .background(Ink.panel)
        .environment(\.colorScheme, .dark)
    }
}

// MARK: - Scene

private struct SceneArea: View {
    let model: AppModel
    var stills: StillFrames?
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
            .accessibilityLabel("Scene")

            SelectionChip(readout: model.stage.readout, meters: model.stage.meters, showHint: hovering || stills != nil)
                .padding(Grid.x3)
        }
        .frame(width: Popover.width, height: Popover.sceneHeight)
        .clipped()
        .onHover { hovering = $0 }
    }
}

/// The selected thing, where it is and how to move it; with nothing selected, how to select.
private struct SelectionChip: View {
    @ObservedObject var readout: SceneReadout
    @ObservedObject var meters: Meters
    let showHint: Bool

    var body: some View {
        Group {
            if let s = readout.selection {
                VStack(alignment: .leading, spacing: Grid.x1) {
                    HStack(spacing: Grid.x2) {
                        IconView(icon: s.kind == .tree ? .tree : s.kind == .bird ? .bird : .river, color: Ink.paper, size: 15)
                        Text(name(s.kind)).font(Ink.text(Ink.Size.title)).foregroundStyle(Ink.paper)
                        Text(coordinates(s.position, withHeight: s.kind == .bird))
                            .font(Ink.text(Ink.Size.caption))
                            .foregroundStyle(Ink.graphite)
                    }
                    Text(!s.movable ? "Flying on its own. Land to move it."
                         : s.kind == .bird ? "← → ↑ ↓ move   ⇧↑ ⇧↓ height   esc let go"
                         : "← → ↑ ↓ move   esc let go")
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.graphite)
                }
                .chip()
            } else if showHint {
                Text("Click a tree, a bird or the river to move it")
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
        case .tree: return "Tree"
        case .bird: return "Bird"
        case .river: return "River"
        }
    }

    /// Relative to the listener. Trees and the river stand on the ground, so only birds get a height.
    private func coordinates(_ p: SIMD3<Float>, withHeight: Bool) -> String {
        func f(_ v: Float) -> String { String(format: "%.1f", v) }
        let ahead = p.x >= 0 ? "\(f(p.x)) m ahead" : "\(f(-p.x)) m behind"
        let side = p.y >= 0 ? "\(f(p.y)) m left" : "\(f(-p.y)) m right"
        return withHeight ? "\(ahead), \(side), \(f(p.z)) m up" : "\(ahead), \(side)"
    }
}

private extension View {
    /// A small dark plate that keeps text legible over the scene.
    func chip() -> some View {
        padding(.horizontal, Grid.x2 + 2)
            .padding(.vertical, Grid.x1 + 2)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Ink.panel.opacity(0.86)))
    }
}

// MARK: - Transport

/// Play, volume, flight, recenter, head tracking; the status text on the right names whatever
/// the pointer is over, with its key.
private struct Transport: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    @ObservedObject var labels: SceneLabels
    @ObservedObject var meters: Meters
    @ObservedObject var controls: ControlsModel
    @State private var hint: String?

    var body: some View {
        HStack(spacing: Grid.x1) {
            ToolButton(icon: audio.isPlaying ? .pause : .play, name: audio.isPlaying ? "Pause" : "Play", key: "space",
                       style: audio.isPlaying ? .plain : .primary, hint: $hint, action: act { model.togglePlay() })
            if let volume = controls.macros.first(where: { Int($0.group) == ASMR_GROUP_MASTER }) {
                VolumeSlider(control: volume, hint: $hint) { v in
                    model.stage.poke()
                    audio.setControl(volume.id, v)
                    model.stage.refreshControls()
                }
                .padding(.trailing, Grid.x2)
            }
            ToolButton(icon: .flight, name: audio.flying ? "Land" : "Fly over the forest", key: "F",
                       style: audio.flying ? .on : .plain, live: meters.live, hint: $hint, action: act { audio.flying.toggle() })
            ToolButton(icon: .headphones, name: head.enabled ? "Turn head tracking off" : "Turn head tracking on", key: "H",
                       style: head.enabled ? .on : .plain, hint: $hint, action: act { head.enabled.toggle() })
            ToolButton(icon: .recenter, name: "Recenter: look ahead now", key: "C", style: .plain, hint: $hint, action: act { head.recenter() })
                .disabled(!head.enabled)
            Spacer(minLength: Grid.x2)
            status
            ToolButton(icon: .power, name: "Quit ASMR X3000", key: "⌘Q", style: .quiet, hint: $hint) { NSApp.terminate(nil) }
                .keyboardShortcut("q", modifiers: .command)
                .padding(.leading, Grid.x1)
        }
        .padding(.horizontal, Grid.x3)
        .frame(height: 48)
    }

    /// Runs a button's action and gives the keyboard back to the scene, so arrows keep moving things.
    private func act(_ f: @escaping () -> Void) -> () -> Void {
        { f(); model.stage.focusScene?() }
    }

    /// The hovered control's name, or the hour and where the head is turned.
    private var status: some View {
        HStack(spacing: Grid.x2) {
            if let hint {
                Text(hint).foregroundStyle(Ink.paper)
            } else {
                if head.enabled {
                    Circle()
                        .fill(Ink.violet.opacity(labels.gaze == nil ? 0.35 : 1))
                        .frame(width: 5, height: 5)
                    Text(labels.gaze.map { "looking \($0)" } ?? (head.connected ? "head tracking" : "no AirPods"))
                        .foregroundStyle(Ink.graphite)
                }
                Text(labels.time).foregroundStyle(Ink.ash)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(1)
        .accessibilityHidden(true)
    }
}

/// A 32 pt square button: visible hit area on hover, pressed and focus states; `.primary` is
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
            IconView(icon: icon, color: iconColor, size: 18, animating: hovering && live)
                .frame(width: 32, height: 32)
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
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(fill(configuration.isPressed)))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(style == .on ? Ink.hairline.opacity(1.6) : .clear, lineWidth: 1))
            .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous)
                .strokeBorder(focused ? Ink.paper.opacity(0.7) : .clear, lineWidth: 1.5)
                .padding(-2))
    }

    private func fill(_ pressed: Bool) -> Color {
        switch style {
        case .primary: return pressed ? Ink.graphite : hovering ? Color(white: 1) : Ink.paper
        case .on: return pressed ? Ink.pressed : hovering ? Ink.pressed : Ink.raised
        case .plain, .quiet: return pressed ? Ink.pressed : hovering ? Ink.raised : .clear
        }
    }
}

/// The master level, small, next to play.
private struct VolumeSlider: View {
    let control: Control
    @Binding var hint: String?
    let set: (Float) -> Void
    @State private var hovering = false
    @State private var dragging = false

    var body: some View {
        HStack(spacing: Grid.x1) {
            IconView(icon: .gain, color: hovering || dragging ? Ink.paper : Ink.graphite, size: 16)
            GeometryReader { geo in
                let w = geo.size.width, x = CGFloat(control.fraction) * w
                ZStack(alignment: .leading) {
                    Capsule().fill(Ink.hairline).frame(height: 2)
                    Capsule().fill(Ink.graphite).frame(width: x, height: 2)
                    RoundedRectangle(cornerRadius: 1)
                        .fill(Ink.paper)
                        .frame(width: hovering || dragging ? 3 : 2, height: 12)
                        .offset(x: min(max(x - 1, 0), w - 2))
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { g in
                        dragging = true
                        set(control.value(atFraction: Float(g.location.x / w)))
                        hint = "Volume   \(Int((control.fraction * 100).rounded())) %"
                    }
                    .onEnded { _ in dragging = false; if !hovering { hint = nil } })
            }
            .frame(width: 64, height: 32)
        }
        .onHover { h in
            hovering = h
            hint = h ? "Volume   \(Int((control.fraction * 100).rounded())) %" : (dragging ? hint : nil)
        }
        .accessibilityElement()
        .accessibilityLabel("Volume")
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
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
            .accessibilityLabel("Spectrogram")

            if open {
                ForEach([100, 1000, 10_000], id: \.self) { hz in
                    let y = yFor(Float(hz), h: h)
                    if y > 7 && y < h - 7 {
                        Text(hz >= 1000 ? "\(hz / 1000) kHz" : "\(hz) Hz")
                            .font(Ink.text(Ink.Size.caption - 1))
                            .foregroundStyle(Ink.ash)
                            .fixedSize()
                            .position(x: 26, y: CGFloat(y))
                            .allowsHitTesting(false)
                    }
                }
                if let p = hover ?? stills?.spectrogramHover {
                    Text(readout(at: p, w: Float(w), h: Float(h)))
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.paper)
                        .padding(.horizontal, Grid.x1 + 2)
                        .padding(.vertical, 1)
                        .background(RoundedRectangle(cornerRadius: 3).fill(Ink.panel.opacity(0.85)))
                        .frame(maxWidth: .infinity, alignment: .trailing)
                        .padding(.top, Grid.x1)
                        .padding(.trailing, 30)
                        .allowsHitTesting(false)
                }
            } else {
                // Closed, the whole strip is one button.
                Button { openStored = true } label: { Color.clear.contentShape(Rectangle()) }
                    .buttonStyle(.plain)
                    .frame(width: w, height: h)
                    .accessibilityLabel("Open the spectrogram")
            }

            Button { openStored.toggle() } label: {
                IconView(icon: .chevron, color: hovering ? Ink.paper : Ink.ash, size: 14)
                    .rotationEffect(.degrees(open ? -90 : 90))
                    .frame(width: 22, height: 22)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .padding(.top, open ? 2 : 1)
            .accessibilityLabel(open ? "Close the spectrogram" : "Open the spectrogram")
        }
        .frame(width: w, height: h)
        .onHover { hovering = $0 }
        .help(open ? "Click to hold, scroll to zoom, double-click to reset" : "Spectrogram")
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
        let freq = hz >= 1000 ? String(format: "%.2f kHz", hz / 1000) : String(format: "%.0f Hz", hz)
        let level = "\(db) dB".replacingOccurrences(of: "-", with: "−")
        return "\(freq)  \(level)\(frozen ? "  held" : "")"
    }
}
