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
    static let rowHeight: CGFloat = 26
    static let iconColumn: CGFloat = 34

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

                SelectionReadout(readout: model.stage.readout, meters: model.stage.meters)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .padding(.leading, 18)
                    .padding(.bottom, 12)
                    .allowsHitTesting(false)
            }
            .frame(width: Self.width, height: Self.sceneHeight)

            Ornament()
                .padding(.horizontal, 16)
                .padding(.vertical, 8)

            SpectrogramPanel(stage: model.stage, stills: stills)
                .padding(.horizontal, 16)

            Tracks(model: model, audio: audio, stills: stills)
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
                    Text(coordinates(s.position, river: s.kind == .river))
                        .font(Ink.italic(14))
                        .foregroundStyle(Ink.paper)
                }
                Text(s.movable ? "Arrows move it; with shift, ↑ and ↓ raise and lower it; esc lets go."
                               : "It is flying on its own; bring flight to rest to perch it.")
                    .font(Ink.text(11.5))
                    .foregroundStyle(Ink.graphite)
            }
        } else {
            Text("Click a tree, a bird or the river to move it; drag to turn the view.")
                .font(Ink.text(11.5))
                .foregroundStyle(Ink.graphite)
        }
    }

    private func coordinates(_ p: SIMD3<Float>, river: Bool) -> String {
        func f(_ v: Float) -> String { String(format: "%.2f", v) }
        let ahead = p.x >= 0 ? "\(f(p.x)) m ahead" : "\(f(-p.x)) m behind"
        let side = p.y >= 0 ? "\(f(p.y)) m left" : "\(f(-p.y)) m right"
        return river ? "\(ahead), \(side)" : "\(ahead), \(side), \(f(p.z)) m up"
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

/// One row per sound: its icon, then its live waveform. Up to the knob the waveform is
/// developed; past it only a latent trace remains. Drag anywhere on a row to set it.
private struct Tracks: View {
    @ObservedObject var model: AppModel
    @ObservedObject var audio: AudioEngine
    var stills: StillFrames?

    private var values: [Float] { [audio.water, audio.wind, audio.birds, audio.orbit, audio.volume] }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(spacing: 0) {
                ForEach(Track.allCases, id: \.self) { t in
                    TrackIcon(track: t, meters: model.stage.meters)
                        .frame(height: Popover.rowHeight)
                }
            }
            .frame(width: Popover.iconColumn, alignment: .leading)

            ZStack(alignment: .topLeading) {
                Group {
                    if let stills {
                        Image(decorative: stills.tracks, scale: 2).resizable()
                    } else {
                        TracksSurface(stage: model.stage, values: values)
                    }
                }
                VStack(spacing: 0) {
                    ForEach(Track.allCases, id: \.self) { t in
                        TrackRow(track: t, value: binding(t), poke: model.stage.poke)
                            .frame(height: Popover.rowHeight)
                    }
                }
            }
            .frame(height: Popover.rowHeight * CGFloat(Track.allCases.count))
        }
    }

    private func binding(_ t: Track) -> Binding<Float> {
        switch t {
        case .water: return $audio.water
        case .wind: return $audio.wind
        case .birds: return $audio.birds
        case .flight: return $audio.orbit
        case .volume: return $audio.volume
        }
    }
}

extension Track {
    var icon: Icon {
        switch self {
        case .water: return .water
        case .wind: return .wind
        case .birds: return .birds
        case .flight: return .flight
        case .volume: return .volume
        }
    }

    var name: String {
        switch self {
        case .water: return "River"
        case .wind: return "Wind"
        case .birds: return "Birdsong"
        case .flight: return "Bird flight"
        case .volume: return "Volume"
        }
    }
}

private struct TrackIcon: View {
    let track: Track
    @ObservedObject var meters: Meters

    var body: some View {
        let audible = meters.audible.contains(track)
        IconView(icon: track.icon, color: audible ? Ink.paper : Ink.graphite, size: 18, level: audible ? 0.6 : 0,
                 animating: meters.live && audible)
    }
}

private struct TrackRow: View {
    let track: Track
    @Binding var value: Float
    var poke: () -> Void = {}
    @State private var hovering = false
    @State private var dragging = false

    var body: some View {
        GeometryReader { geo in
            let x = CGFloat(value) * geo.size.width
            ZStack(alignment: .topLeading) {
                Color.clear.contentShape(Rectangle())
                // The index: a hairline with a lozenge, as on an engraved scale.
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
                    poke()
                    value = Float(min(max(g.location.x / geo.size.width, 0), 1))
                }
                .onEnded { _ in dragging = false })
        }
        .onHover { hovering = $0 }
        .accessibilityElement()
        .accessibilityLabel(track.name)
        .accessibilityValue("\(Int(value * 100)) %")
        .accessibilityAdjustableAction { dir in
            value = min(max(value + (dir == .increment ? 0.05 : -0.05), 0), 1)
        }
    }
}
