import CAsmr
import SwiftUI

/// Pre-rendered Metal frames, used instead of live views when writing a snapshot.
struct StillFrames {
    var scene: CGImage
    var spectrogram: CGImage
    var tracks: CGImage
    var spectrogramHover: SIMD2<Float>?
}

/// The popover: the engraved scene is the page; the spectrogram and the tracks sit under it.
struct Popover: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    var stills: StillFrames?

    static let width: CGFloat = 560
    static let sceneHeight: CGFloat = 356
    static let spectrogramHeight: CGFloat = 104
    static let rowHeight: CGFloat = 23
    static let iconColumn: CGFloat = 34
    static let maxTracksHeight: CGFloat = 300

    init(model: AppModel, stills: StillFrames? = nil) {
        self.model = model
        audio = model.audio
        head = model.head
        self.stills = stills
    }

    var body: some View {
        VStack(spacing: 0) {
            ZStack(alignment: .topLeading) {
                Group {
                    if let stills {
                        Image(decorative: stills.scene, scale: 2).resizable()
                    } else {
                        SceneSurface(stage: model.stage)
                    }
                }
                .frame(width: Self.width, height: Self.sceneHeight)
                .accessibilityLabel("Scene")

                LinearGradient(colors: [Ink.black.opacity(0), Ink.black.opacity(0.9)], startPoint: .top, endPoint: .bottom)
                    .frame(height: 70)
                    .frame(maxHeight: .infinity, alignment: .bottom)
                    .allowsHitTesting(false)

                RadialGradient(colors: [Ink.black.opacity(0.85), Ink.black.opacity(0)], center: .center, startRadius: 0, endRadius: 80)
                    .frame(width: 220, height: 90)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .offset(x: 50, y: -20)
                    .allowsHitTesting(false)

                Transport(model: model, audio: audio, head: head, meters: model.stage.meters)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.top, 10)
                    .padding(.trailing, 10)

                OrientationLabels(labels: model.stage.labels)
                    .allowsHitTesting(false)

                SelectionReadout(readout: model.stage.readout, meters: model.stage.meters)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .padding(.leading, 18)
                    .padding(.bottom, 12)
                    .allowsHitTesting(false)
            }
            .frame(width: Self.width, height: Self.sceneHeight)

            PromptField(prompt: model.prompt)
                .padding(.horizontal, 18)
                .padding(.top, 10)

            Ornament()
                .padding(.horizontal, 16)
                .padding(.vertical, 8)

            SpectrogramPanel(stage: model.stage, stills: stills)
                .padding(.horizontal, 16)

            TracksPanel(model: model, controls: model.stage.controls, stills: stills)
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 14)
        }
        .frame(width: Self.width)
        .background(Ink.black)
        .environment(\.colorScheme, .dark)
    }
}

/// A hairline with a small lozenge: the one ornament.
private struct Ornament: View {
    var body: some View {
        HStack(spacing: 8) {
            Rectangle().fill(Ink.ash).frame(height: 0.5)
            Rectangle().stroke(Ink.graphite, lineWidth: 0.75).frame(width: 4.5, height: 4.5).rotationEffect(.degrees(45))
            Rectangle().fill(Ink.ash).frame(height: 0.5)
        }
        .frame(height: 6)
        .accessibilityHidden(true)
    }
}

// MARK: - Transport

private struct Transport: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    @ObservedObject var head: HeadTracker
    @ObservedObject var meters: Meters

    var body: some View {
        HStack(spacing: 0) {
            QuietButton(icon: audio.isPlaying ? .pause : .play, name: audio.isPlaying ? "Pause" : "Play",
                        emphasised: !audio.isPlaying, live: meters.live) { model.togglePlay() }
            QuietButton(icon: .recenter, name: "Recenter", live: meters.live) { head.recenter() }
            QuietButton(icon: .headphones, name: head.enabled ? "Turn head tracking off" : "Turn head tracking on",
                        struck: !head.enabled, live: meters.live) { head.enabled.toggle() }
            QuietButton(icon: .quit, name: "Quit", live: meters.live) { NSApp.terminate(nil) }
        }
    }
}

private struct QuietButton: View {
    let icon: Icon
    let name: String
    var struck = false
    var emphasised = false
    var live = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            IconView(icon: icon, color: hovering || emphasised ? Ink.paper : Ink.graphite, size: 17,
                     animating: hovering && live, struck: struck)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .accessibilityLabel(name)
    }
}

// MARK: - Orientation

/// "ahead" on the ground in front of the listener; the hour and where the head is turned in the
/// corner. The gaze line carries a small violet point: the same colour as the gaze in the scene.
private struct OrientationLabels: View {
    @ObservedObject var labels: SceneLabels

    var body: some View {
        ZStack(alignment: .topLeading) {
            if let a = labels.ahead {
                Text("ahead")
                    .font(Ink.text(10))
                    .foregroundStyle(Ink.graphite)
                    .padding(.horizontal, 3)
                    .background(Ink.black.opacity(0.7))
                    .fixedSize()
                    .position(x: a.x, y: a.y + 9)
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(labels.time)
                    .font(Ink.text(11))
                    .foregroundStyle(Ink.graphite)
                HStack(spacing: 6) {
                    Circle()
                        .fill(Color(red: 0.62, green: 0.45, blue: 1.0).opacity(labels.gaze == nil ? 0.4 : 1))
                        .frame(width: 5, height: 5)
                        .shadow(color: Color(red: 0.56, green: 0.36, blue: 1.0), radius: labels.gaze == nil ? 0 : 3)
                    Text(labels.gaze.map { "looking \($0)" } ?? "head tracking off")
                        .font(Ink.text(11))
                        .foregroundStyle(Ink.graphite)
                }
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .background(Ink.black.opacity(0.65))
            .padding(.leading, 10)
            .padding(.top, 9)
        }
        .frame(width: Popover.width, height: Popover.sceneHeight, alignment: .topLeading)
    }
}

// MARK: - Selection readout

private struct SelectionReadout: View {
    @ObservedObject var readout: SceneReadout
    @ObservedObject var meters: Meters

    var body: some View {
        if let s = readout.selection {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    IconView(icon: s.kind == .tree ? .tree : s.kind == .bird ? .bird : .river, size: 16,
                             animating: meters.live)
                    Text(name(s.kind)).font(Ink.text(14)).foregroundStyle(Ink.paper)
                    Text(coordinates(s.position, withHeight: s.kind == .bird))
                        .font(Ink.italic(12.5))
                        .foregroundStyle(Ink.graphite)
                }
                Text(!s.movable ? "It is flying on its own; bring flight to rest to perch it."
                     : s.kind == .bird ? "Arrows move it; with shift, ↑ and ↓ raise and lower it; esc lets go."
                     : "Arrows move it along the ground; esc lets go.")
                    .font(Ink.text(11.5))
                    .foregroundStyle(Ink.graphite)
            }
        } else {
            Text("Click a tree, a bird or the river to move it; drag to turn the view.")
                .font(Ink.text(11.5))
                .foregroundStyle(Ink.graphite)
        }
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

// MARK: - Spectrogram

private struct SpectrogramPanel: View {
    let stage: Stage
    var stills: StillFrames?
    @State private var hover: SIMD2<Float>?
    @State private var zoom = SIMD2<Float>(0, 1)
    @State private var frozen = false

    var body: some View {
        let h = Popover.spectrogramHeight
        let w = Popover.width - 32
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
            .accessibilityLabel("Spectrogram")

            ForEach([100, 1000, 10_000], id: \.self) { hz in
                let y = yFor(Float(hz), h: h)
                if y > 7 && y < h - 7 {
                    Text(hz >= 1000 ? "\(hz / 1000) kHz" : "\(hz) Hz")
                        .font(Ink.italic(10.5))
                        .foregroundStyle(Ink.graphite)
                        .padding(.horizontal, 4)
                        .background(Ink.black)
                        .fixedSize()
                        .position(x: 24, y: CGFloat(y))
                }
            }

            if let p = hover ?? stills?.spectrogramHover {
                Text(readout(at: p, w: Float(w), h: Float(h)))
                    .font(Ink.italic(12))
                    .foregroundStyle(Ink.paper)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(Ink.black)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.top, 2)
                    .padding(.trailing, 14)
                    .allowsHitTesting(false)
            }
        }
        .frame(width: w, height: h)
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
        let ago = String(format: "%.1f s ago", Double(back) / Stage.columnsPerSecond)
        let level = "\(db) dB".replacingOccurrences(of: "-", with: "−")
        return "\(freq), \(level), \(ago)\(frozen ? ", held" : "")"
    }
}

// MARK: - Tracks

/// A long branch (water has nine parameters) scrolls rather than growing the popover.
private struct TracksPanel: View {
    let model: AppModel
    @ObservedObject var controls: ControlsModel
    var stills: StillFrames?

    var body: some View {
        let height = min(Tracks.contentHeight(controls), Popover.maxTracksHeight)
        if stills != nil {
            // ImageRenderer does not draw ScrollView content; a snapshot shows the top of the list.
            Tracks(model: model, stills: stills)
                .frame(height: height, alignment: .top)
                .clipped()
        } else {
            ScrollView(.vertical, showsIndicators: false) {
                Tracks(model: model, stills: stills)
            }
            .frame(height: height)
        }
    }
}

/// The engine's control tree as branching tracks. A macro row is its group's pictogram and the
/// live waveform of that sound, developed up to the knob. Clicking the pictogram opens the
/// branch: the physical parameters underneath as fine sliders, joined by hairlines.
struct Tracks: View {
    @ObservedObject var model: AppModel
    @ObservedObject var controls: ControlsModel
    var stills: StillFrames?

    static let childHeight: CGFloat = 20

    init(model: AppModel, stills: StillFrames?) {
        self.model = model
        controls = model.stage.controls
        self.stills = stills
    }

    var body: some View {
        let macros = controls.macros
        let hour = controls.controls.first { $0.key == "env.time_of_day" }?.value
        ZStack(alignment: .topLeading) {
            Group {
                if let stills {
                    Image(decorative: stills.tracks, scale: 2).resizable()
                } else {
                    TracksSurface(stage: model.stage, rows: scopeRows(macros))
                }
            }
            .padding(.leading, Popover.iconColumn)

            VStack(spacing: 0) {
                ForEach(macros, id: \.id) { m in
                    let kids = controls.children(of: m.id)
                    let open = controls.expanded == m.id && !kids.isEmpty
                    MacroRow(control: m, hour: hour, hasChildren: !kids.isEmpty, open: open, meters: model.stage.meters,
                             set: set, toggle: { toggle(m.id) })
                        .frame(height: Popover.rowHeight)
                    if open {
                        ForEach(Array(kids.enumerated()), id: \.element.id) { i, c in
                            ChildRow(control: c, hour: hour, last: i == kids.count - 1, set: set)
                                .frame(height: Self.childHeight)
                        }
                    }
                }
            }
        }
        .frame(height: height(macros))
    }

    static func layout(_ macros: [Control], expanded: UInt32?, children: (UInt32) -> Int) -> [ScopeRow] {
        var y: Float = 0
        var rows: [ScopeRow] = []
        for m in macros {
            rows.append(ScopeRow(top: y, height: Float(Popover.rowHeight), track: Track(group: m.group), fraction: m.fraction))
            y += Float(Popover.rowHeight)
            if expanded == m.id { y += Float(childHeight) * Float(children(m.id)) }
        }
        return rows
    }

    private func scopeRows(_ macros: [Control]) -> [ScopeRow] {
        Self.layout(macros, expanded: controls.expanded) { controls.children(of: $0).count }
    }

    private func height(_ macros: [Control]) -> CGFloat { Self.contentHeight(controls) }

    static func contentHeight(_ controls: ControlsModel) -> CGFloat {
        let open = controls.expanded.map { controls.children(of: $0).count } ?? 0
        return Popover.rowHeight * CGFloat(controls.macros.count) + childHeight * CGFloat(open)
    }

    private func set(_ id: UInt32, _ value: Float) {
        model.stage.poke()
        model.audio.setControl(id, value)
        model.stage.refreshControls()
    }

    private func toggle(_ id: UInt32) {
        controls.expanded = controls.expanded == id ? nil : id
    }
}

extension Track {
    init?(group: UInt32) {
        switch Int(group) {
        case ASMR_GROUP_WATER: self = .water
        case ASMR_GROUP_WIND, ASMR_GROUP_FOLIAGE: self = .wind
        case ASMR_GROUP_BIRDS: self = .birds
        case ASMR_GROUP_MASTER: self = .volume
        case ASMR_GROUP_RAIN: self = .rain
        case ASMR_GROUP_INSECTS: self = .insects
        case ASMR_GROUP_FIRE: self = .fire
        default: return nil   // environment shapes every sound; it has no waveform of its own
        }
    }
}

private struct MacroRow: View {
    let control: Control
    var hour: Float?
    let hasChildren: Bool
    let open: Bool
    @ObservedObject var meters: Meters
    let set: (UInt32, Float) -> Void
    let toggle: () -> Void
    @State private var hovering = false
    @State private var dragging = false

    var body: some View {
        let track = Track(group: control.group)
        let audible = track.map { meters.audible.contains($0) } ?? false
        HStack(spacing: 0) {
            Button(action: toggle) {
                ZStack(alignment: .bottomTrailing) {
                    IconView(icon: Icon.forControl(control, hour: hour), color: audible || open ? Ink.paper : Ink.graphite, size: 18,
                             level: audible ? 0.6 : 0, animating: meters.live && audible)
                        .frame(width: 22, height: 22)
                    if hasChildren {
                        // The branch mark: a small lozenge, filled while the branch is open.
                        Rectangle()
                            .fill(open ? Ink.paper : Ink.black)
                            .overlay(Rectangle().stroke(Ink.graphite, lineWidth: 0.75))
                            .frame(width: 3.5, height: 3.5)
                            .rotationEffect(.degrees(45))
                            .offset(x: 5, y: 1)
                    }
                }
                .frame(width: Popover.iconColumn, height: Popover.rowHeight, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .disabled(!hasChildren)
            .accessibilityLabel(hasChildren ? (open ? "Close \(control.key) details" : "Open \(control.key) details") : control.key)

            GeometryReader { geo in
                let x = CGFloat(control.fraction) * geo.size.width
                ZStack(alignment: .topLeading) {
                    Color.clear.contentShape(Rectangle())
                    Rectangle()
                        .fill(Ink.paper)
                        .frame(width: 1, height: geo.size.height - 6)
                        .position(x: x, y: geo.size.height / 2 + 2)
                    Rectangle()
                        .fill(hovering || dragging ? Ink.paper : Ink.black)
                        .overlay(Rectangle().stroke(Ink.paper, lineWidth: 1))
                        .frame(width: 5, height: 5)
                        .rotationEffect(.degrees(45))
                        .position(x: x, y: 3.5)
                }
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { g in
                        dragging = true
                        set(control.id, control.value(atFraction: Float(g.location.x / geo.size.width)))
                    }
                    .onEnded { _ in dragging = false })
            }
            .onHover { hovering = $0 }
            .accessibilityElement()
            .accessibilityLabel(control.key)
            .accessibilityValue("\(Int(control.fraction * 100)) %")
            .accessibilityAdjustableAction { dir in
                set(control.id, control.value(atFraction: control.fraction + (dir == .increment ? 0.05 : -0.05)))
            }
        }
    }
}

/// A physical parameter under its macro: branch hairlines, a small pictogram, and a fine slider
/// (or, for presets, a row of stops). Its name, value and unit appear only on hover.
private struct ChildRow: View {
    let control: Control
    var hour: Float?
    let last: Bool
    let set: (UInt32, Float) -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 0) {
            ZStack(alignment: .leading) {
                Canvas { ctx, size in
                    // The branch: down from the macro's pictogram, then across to this child.
                    let x: CGFloat = 10.5, mid = size.height / 2
                    var p = Path()
                    p.move(to: CGPoint(x: x, y: 0))
                    p.addLine(to: CGPoint(x: x, y: last ? mid : size.height))
                    p.move(to: CGPoint(x: x, y: mid))
                    p.addQuadCurve(to: CGPoint(x: 17, y: mid), control: CGPoint(x: x, y: mid))
                    ctx.stroke(p, with: .color(Ink.graphite.opacity(0.7)), lineWidth: 0.6)
                }
                IconView(icon: Icon.forControl(control, hour: hour), color: hovering ? Ink.paper : Ink.graphite, size: 13, lineWidth: 1)
                    .offset(x: 18)
            }
            .frame(width: Popover.iconColumn)

            GeometryReader { geo in
                ZStack(alignment: .trailing) {
                    if control.isDiscrete {
                        Stops(control: control, width: geo.size.width, set: set)
                    } else {
                        FineSlider(control: control, width: geo.size.width, hovering: hovering, set: set)
                    }
                    if hovering {
                        Text(label)
                            .font(Ink.italic(11.5))
                            .foregroundStyle(Ink.paper)
                            .padding(.horizontal, 5)
                            .background(Ink.black)
                            .allowsHitTesting(false)
                    }
                }
            }
        }
        .onHover { hovering = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(control.key)
        .accessibilityValue(label)
        .accessibilityAdjustableAction { dir in
            let step: Float = control.isDiscrete ? 1 / max(control.max - control.min, 1) : 0.05
            set(control.id, control.value(atFraction: control.fraction + (dir == .increment ? step : -step)))
        }
    }

    private var label: String {
        let index = Int(control.value.rounded())
        if control.isDiscrete, index >= 0, index < control.options.count, !control.options[index].isEmpty {
            return "\(control.key) \(control.options[index])"
        }
        let v = control.isDiscrete ? String(index)
            : abs(control.value) >= 10 ? String(format: "%.1f", control.value) : String(format: "%.2f", control.value)
        return control.unit.isEmpty ? "\(control.key) \(v)" : "\(control.key) \(v) \(control.unit)"
    }
}

/// A hairline scale with tenths ticked; the part up to the knob is drawn in ink.
private struct FineSlider: View {
    let control: Control
    let width: CGFloat
    let hovering: Bool
    let set: (UInt32, Float) -> Void

    var body: some View {
        let x = CGFloat(control.fraction) * width
        Canvas { ctx, size in
            let mid = size.height / 2
            var ticks = Path()
            for i in 0...10 {
                let tx = size.width * CGFloat(i) / 10
                ticks.addRect(CGRect(x: tx - 0.25, y: mid - (i % 5 == 0 ? 2.5 : 1.5), width: 0.5, height: i % 5 == 0 ? 5 : 3))
            }
            ctx.fill(ticks, with: .color(Ink.ash))
            ctx.fill(Path(CGRect(x: 0, y: mid - 0.25, width: size.width, height: 0.5)), with: .color(Ink.ash))
            ctx.fill(Path(CGRect(x: 0, y: mid - 0.5, width: x, height: 1)), with: .color(Ink.paper.opacity(0.85)))
            // Knob: a small lozenge on a short hairline.
            ctx.fill(Path(CGRect(x: x - 0.5, y: mid - 5, width: 1, height: 10)), with: .color(Ink.paper))
            var lz = Path()
            lz.move(to: CGPoint(x: x, y: mid - 3))
            lz.addLine(to: CGPoint(x: x + 3, y: mid))
            lz.addLine(to: CGPoint(x: x, y: mid + 3))
            lz.addLine(to: CGPoint(x: x - 3, y: mid))
            lz.closeSubpath()
            // Filled once the user has set it; hollow while it follows its macro.
            ctx.fill(lz, with: .color(hovering || control.isOverridden ? Ink.paper : Ink.black))
            ctx.stroke(lz, with: .color(Ink.paper), lineWidth: 0.8)
        }
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 0).onChanged { g in
            set(control.id, control.value(atFraction: Float(g.location.x / width)))
        })
        // Double-click hands the parameter back to its macro.
        .simultaneousGesture(TapGesture(count: 2).onEnded { set(control.id, .nan) })
    }
}

/// Presets: a dotted rule with one stop per preset; the chosen one is filled.
private struct Stops: View {
    let control: Control
    let width: CGFloat
    let set: (UInt32, Float) -> Void

    var body: some View {
        let n = max(Int((control.max - control.min).rounded()) + 1, 2)
        let chosen = Int((control.value - control.min).rounded())
        ZStack(alignment: .leading) {
            Canvas { ctx, size in
                let mid = size.height / 2
                var dots = Path()
                var x: CGFloat = 0
                while x < size.width { dots.addRect(CGRect(x: x, y: mid - 0.25, width: 1, height: 0.5)); x += 3 }
                ctx.fill(dots, with: .color(Ink.ash))
            }
            ForEach(0..<n, id: \.self) { i in
                let x = width * CGFloat(i) / CGFloat(n - 1)
                Button { set(control.id, control.min + Float(i)) } label: {
                    Rectangle()
                        .fill(i == chosen ? Ink.paper : Ink.black)
                        .overlay(Rectangle().stroke(i == chosen ? Ink.paper : Ink.graphite, lineWidth: 0.9))
                        .frame(width: 6, height: 6)
                        .rotationEffect(.degrees(45))
                        .frame(width: 18, height: 18)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .position(x: min(max(x, 4), width - 4), y: Tracks.childHeight / 2)
            }
        }
    }
}
