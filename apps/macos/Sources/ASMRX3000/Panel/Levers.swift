import CAsmr
import SwiftUI

/// The engine's control tree as a few macro levers. Each lever is its group's pictogram and a
/// slider whose fill is the live waveform of that sound. The pictogram (or the chevron) opens
/// the group's drawer: its physical parameters, named, one group at a time.
///
/// Generic over the tree: the groups listed in `primary` come first (flight joins them while
/// flying); every other group, including ones added to the engine later, waits behind "more".
struct LeverPanel: View {
    let model: AppModel
    @ObservedObject var controls: ControlsModel
    @ObservedObject var audio: AudioEngine
    var stills: StillFrames?
    @AppStorage("levers.showAll") private var showAllStored = false

    static let primary = [ASMR_GROUP_WATER, ASMR_GROUP_WIND, ASMR_GROUP_BIRDS, ASMR_GROUP_RAIN, ASMR_GROUP_ENVIRONMENT]
    /// Lives in the transport row instead.
    static let elsewhere = [ASMR_GROUP_MASTER]

    init(model: AppModel, controls: ControlsModel, stills: StillFrames?) {
        self.model = model
        self.controls = controls
        audio = model.audio
        self.stills = stills
    }

    private var showAll: Bool { stills.map(\.showAll) ?? showAllStored }
    private var drawer: UInt32? { stills != nil ? stills?.drawer : controls.expanded }

    /// The levers in display order: (always shown, behind "more").
    static func split(_ macros: [Control], flying: Bool) -> (shown: [Control], more: [Control]) {
        var order = primary
        if flying { order.insert(ASMR_GROUP_FLIGHT, at: 0) }
        let rest = macros.filter { !elsewhere.contains(Int($0.group)) }
        var shown = order.compactMap { g in rest.first { Int($0.group) == g } }
        var more = rest.filter { m in !shown.contains { $0.id == m.id } }
        // Keep at least four levers in view when the engine has few of the usual groups.
        while shown.count < 4, !more.isEmpty { shown.append(more.removeFirst()) }
        return (shown, more)
    }

    static func visible(_ macros: [Control], flying: Bool, showAll: Bool) -> [Control] {
        let s = split(macros, flying: flying)
        return showAll ? s.shown + s.more : s.shown
    }

    static func layout(_ levers: [Control]) -> [ScopeRow] {
        levers.enumerated().map { i, m in
            ScopeRow(top: Float(i) * Float(Popover.leverHeight), height: Float(Popover.leverHeight),
                     track: Track(group: m.group), fraction: m.fraction)
        }
    }

    var body: some View {
        let hour = controls.controls.first { $0.key == "env.time_of_day" }?.value
        if let id = drawer, let macro = controls.controls.first(where: { $0.id == id }) {
            Drawer(macro: macro, children: controls.children(of: id), hour: hour, scrolls: stills == nil,
                   set: set, close: { controls.expanded = nil })
        } else {
            levers(hour: hour)
        }
    }

    private func levers(hour: Float?) -> some View {
        let split = Self.split(controls.macros, flying: audio.flying)
        let shown = showAll ? split.shown + split.more : split.shown
        return VStack(alignment: .leading, spacing: 0) {
            ZStack(alignment: .topLeading) {
                Group {
                    if let stills {
                        Image(decorative: stills.tracks, scale: 2).resizable()
                    } else {
                        TracksSurface(stage: model.stage, rows: Self.layout(shown))
                    }
                }
                .frame(width: Popover.sliderWidth, height: Popover.leverHeight * CGFloat(shown.count))
                .padding(.leading, Popover.iconColumn)

                VStack(spacing: 0) {
                    ForEach(shown, id: \.id) { m in
                        LeverRow(control: m, hour: hour, hasChildren: !controls.children(of: m.id).isEmpty,
                                 meters: model.stage.meters, set: set, open: { controls.expanded = m.id })
                            .frame(height: Popover.leverHeight)
                    }
                }
            }
            if !split.more.isEmpty {
                MoreRow(more: split.more, hour: hour, showAll: showAll) { showAllStored.toggle() }
                    .frame(height: 24)
            }
        }
    }

    private func set(_ id: UInt32, _ value: Float) {
        model.stage.poke()
        model.audio.setControl(id, value)
        model.stage.refreshControls()
    }
}

// MARK: - Naming and values

enum ControlText {
    /// "water.rock_size" → "Rock size"; a macro is named by its group.
    static func name(_ c: Control) -> String {
        if c.isMacro { return group(c) }
        let leaf = c.key.split(separator: ".").last.map(String.init) ?? c.key
        return sentence(leaf.replacingOccurrences(of: "_", with: " "))
    }

    static func group(_ c: Control) -> String {
        switch Int(c.group) {
        case ASMR_GROUP_WATER: return "Water"
        case ASMR_GROUP_WIND: return "Wind"
        case ASMR_GROUP_FOLIAGE: return "Foliage"
        case ASMR_GROUP_RAIN: return "Rain"
        case ASMR_GROUP_BIRDS: return "Birds"
        case ASMR_GROUP_MASTER: return "Volume"
        case ASMR_GROUP_ENVIRONMENT: return "Time and weather"
        case ASMR_GROUP_INSECTS: return "Insects"
        case ASMR_GROUP_FIRE: return "Fire"
        case ASMR_GROUP_FLIGHT: return "Flight"
        default:
            let head = c.key.split(separator: ".").first.map(String.init) ?? c.key
            return sentence(head.replacingOccurrences(of: "_", with: " "))
        }
    }

    static func sentence(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    static func value(_ c: Control) -> String {
        let index = Int(c.value.rounded())
        if c.isDiscrete {
            if index >= 0, index < c.options.count, !c.options[index].isEmpty {
                return c.options[index].replacingOccurrences(of: "_", with: " ")
            }
            return String(index)
        }
        if c.unit == "h" {
            let m = Int((c.value * 60).rounded()) % (24 * 60)
            return String(format: "%02d:%02d", m / 60, m % 60)
        }
        if c.unit.isEmpty {
            return "\(Int((c.fraction * 100).rounded())) %"
        }
        let v = abs(c.value) >= 100 ? String(format: "%.0f", c.value)
            : abs(c.value) >= 10 ? String(format: "%.1f", c.value) : String(format: "%.2f", c.value)
        return "\(v.replacingOccurrences(of: "-", with: "−")) \(c.unit)"
    }
}

// MARK: - Lever

private struct LeverRow: View {
    let control: Control
    var hour: Float?
    let hasChildren: Bool
    @ObservedObject var meters: Meters
    let set: (UInt32, Float) -> Void
    let open: () -> Void
    @State private var hovering = false
    @State private var dragging = false

    var body: some View {
        let track = Track(group: control.group)
        let audible = track.map { meters.audible.contains($0) } ?? false
        HStack(spacing: 0) {
            Button(action: open) {
                IconView(icon: Icon.forControl(control, hour: hour),
                         color: hovering ? Ink.paper : audible ? Ink.paper.opacity(0.85) : Ink.graphite,
                         size: 18, level: audible ? 0.6 : 0, animating: hovering && meters.live)
                    .frame(width: Popover.iconColumn, height: Popover.leverHeight, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .disabled(!hasChildren)
            .help(hasChildren ? "\(ControlText.group(control)): details" : ControlText.group(control))
            .accessibilityLabel(hasChildren ? "Open \(ControlText.group(control)) details" : ControlText.group(control))

            GeometryReader { geo in
                let w = geo.size.width
                let x = CGFloat(control.fraction) * w
                ZStack(alignment: .topLeading) {
                    Color.clear.contentShape(Rectangle())
                    RoundedRectangle(cornerRadius: 1)
                        .fill(Ink.paper)
                        .frame(width: hovering || dragging ? 3 : 2, height: hovering || dragging ? 18 : 14)
                        .position(x: min(max(x, 1.5), w - 1.5), y: geo.size.height / 2)
                    if hovering || dragging {
                        Text("\(ControlText.group(control))  \(ControlText.value(control))")
                            .font(Ink.text(Ink.Size.caption))
                            .foregroundStyle(Ink.paper)
                            .padding(.horizontal, Grid.x1 + 2)
                            .padding(.vertical, 1)
                            .background(RoundedRectangle(cornerRadius: 3).fill(Ink.panel.opacity(0.9)))
                            .fixedSize()
                            .frame(width: w, height: geo.size.height, alignment: x > w * 0.6 ? .leading : .trailing)
                            .allowsHitTesting(false)
                    }
                }
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { g in
                        dragging = true
                        set(control.id, control.value(atFraction: Float(g.location.x / w)))
                    }
                    .onEnded { _ in dragging = false })
            }
            .accessibilityElement()
            .accessibilityLabel(ControlText.group(control))
            .accessibilityValue(ControlText.value(control))
            .accessibilityAdjustableAction { dir in
                set(control.id, control.value(atFraction: control.fraction + (dir == .increment ? 0.05 : -0.05)))
            }

            Button(action: open) {
                IconView(icon: .chevron, color: hovering ? Ink.graphite : Ink.ash, size: 14)
                    .frame(width: Popover.chevronColumn, height: Popover.leverHeight, alignment: .trailing)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .opacity(hasChildren ? 1 : 0)
            .disabled(!hasChildren)
            .accessibilityHidden(true)
        }
        .onHover { hovering = $0 }
    }
}

/// "More": the hidden groups' pictograms; click to show their levers too.
private struct MoreRow: View {
    let more: [Control]
    var hour: Float?
    let showAll: Bool
    let toggle: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: Grid.x2) {
                IconView(icon: showAll ? .back : .more, color: hovering ? Ink.paper : Ink.graphite, size: 16)
                    .rotationEffect(.degrees(showAll ? 90 : 0))
                    .frame(width: Popover.iconColumn - Grid.x2, alignment: .leading)
                if showAll {
                    Text("Fewer").foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                } else {
                    ForEach(more, id: \.id) { m in
                        IconView(icon: Icon.forControl(m, hour: hour), color: hovering ? Ink.graphite : Ink.ash, size: 14,
                                 lineWidth: 1)
                    }
                    Text("\(more.count) more").foregroundStyle(hovering ? Ink.paper : Ink.ash)
                }
                Spacer()
            }
            .font(Ink.text(Ink.Size.caption))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .accessibilityLabel(showAll ? "Show fewer levers" : "Show \(more.count) more levers")
    }
}

// MARK: - Drawer

/// One group's physical parameters, named, with their values. Replaces the levers until closed.
private struct Drawer: View {
    let macro: Control
    let children: [Control]
    var hour: Float?
    let scrolls: Bool
    let set: (UInt32, Float) -> Void
    let close: () -> Void
    @State private var hoveringBack = false

    static let maxHeight: CGFloat = 248

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: Grid.x2) {
                Button(action: close) {
                    HStack(spacing: Grid.x1) {
                        IconView(icon: .back, color: hoveringBack ? Ink.paper : Ink.graphite, size: 14)
                        IconView(icon: Icon.forControl(macro, hour: hour), color: Ink.paper, size: 18)
                        Text(ControlText.group(macro))
                            .font(Ink.text(Ink.Size.title))
                            .foregroundStyle(Ink.paper)
                    }
                    .padding(.trailing, Grid.x2)
                    .frame(height: Popover.childHeight)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .onHover { hoveringBack = $0 }
                .keyboardShortcut(.leftArrow, modifiers: .command)
                .accessibilityLabel("Back to all sounds")
                .help("Back   ⌘←")
                Spacer()
                Text(ControlText.value(macro))
                    .font(Ink.text(Ink.Size.body))
                    .foregroundStyle(Ink.graphite)
            }
            .padding(.leading, -2)

            let rows = VStack(spacing: 0) {
                ForEach(children, id: \.id) { c in
                    ChildRow(control: c, hour: hour, set: set)
                        .frame(height: Popover.childHeight)
                }
            }
            let height = Popover.childHeight * CGFloat(children.count)
            if scrolls && height > Self.maxHeight {
                ScrollView(.vertical) { rows }.frame(height: Self.maxHeight)
            } else {
                rows
            }
        }
    }
}

/// A parameter: small pictogram, its name, a slider (or the presets as words), and the value.
/// The value is bright once the user has set it; dim while it follows the group's lever.
private struct ChildRow: View {
    let control: Control
    var hour: Float?
    let set: (UInt32, Float) -> Void
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 0) {
            IconView(icon: Icon.forControl(control, hour: hour), color: hovering ? Ink.paper : Ink.graphite, size: 14, lineWidth: 1)
                .frame(width: 24, alignment: .leading)
            Text(ControlText.name(control))
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                .lineLimit(1)
                .frame(width: 112, alignment: .leading)
            if control.isDiscrete {
                Presets(control: control, set: set)
            } else {
                GeometryReader { geo in
                    FineSlider(control: control, width: geo.size.width, active: hovering, set: set)
                }
                Text(ControlText.value(control))
                    .font(Ink.text(Ink.Size.body))
                    .foregroundStyle(control.isOverridden ? Ink.paper : Ink.graphite)
                    .lineLimit(1)
                    .frame(width: 84, alignment: .trailing)
            }
        }
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .help(control.isOverridden ? "Double-click the slider to follow \(ControlText.group(control)) again" : "")
        .accessibilityElement(children: .combine)
        .accessibilityLabel(ControlText.name(control))
        .accessibilityValue(ControlText.value(control))
        .accessibilityAdjustableAction { dir in
            let step: Float = control.isDiscrete ? 1 / max(control.max - control.min, 1) : 0.05
            set(control.id, control.value(atFraction: control.fraction + (dir == .increment ? step : -step)))
        }
    }
}

/// A plain rail with the part up to the knob drawn brighter.
private struct FineSlider: View {
    let control: Control
    let width: CGFloat
    let active: Bool
    let set: (UInt32, Float) -> Void

    var body: some View {
        let x = CGFloat(control.fraction) * width
        Canvas { ctx, size in
            let mid = size.height / 2
            ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - 1, width: size.width, height: 2), cornerRadius: 1), with: .color(Ink.hairline))
            ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - 1, width: x, height: 2), cornerRadius: 1),
                     with: .color(control.isOverridden ? Ink.paper.opacity(0.85) : Ink.graphite))
            let kw: CGFloat = active ? 3 : 2, kh: CGFloat = active ? 16 : 12
            ctx.fill(Path(roundedRect: CGRect(x: min(max(x - kw / 2, 0), size.width - kw), y: mid - kh / 2, width: kw, height: kh),
                          cornerRadius: 1), with: .color(Ink.paper))
        }
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 0).onChanged { g in
            set(control.id, control.value(atFraction: Float(g.location.x / width)))
        })
        // Double-click hands the parameter back to its lever.
        .simultaneousGesture(TapGesture(count: 2).onEnded { set(control.id, .nan) })
    }
}

/// Presets as a row of small stops and the chosen one's name; hover a stop to read its name.
private struct Presets: View {
    let control: Control
    let set: (UInt32, Float) -> Void
    @State private var hovered: Int?

    var body: some View {
        let n = max(Int((control.max - control.min).rounded()) + 1, 2)
        let chosen = Int((control.value - control.min).rounded())
        HStack(spacing: 0) {
            ForEach(0..<n, id: \.self) { i in
                Button { set(control.id, control.min + Float(i)) } label: {
                    RoundedRectangle(cornerRadius: 1.5)
                        .fill(i == chosen ? Ink.paper : hovered == i ? Ink.graphite : Ink.hairline.opacity(1.8))
                        .frame(width: 7, height: 7)
                        .frame(width: 16, height: Popover.childHeight)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .onHover { hovered = $0 ? i : (hovered == i ? nil : hovered) }
                .accessibilityLabel(title(i))
                .accessibilityAddTraits(i == chosen ? .isSelected : [])
            }
            Spacer(minLength: Grid.x2)
            Text(title(hovered ?? chosen))
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(hovered == nil || hovered == chosen ? Ink.paper : Ink.graphite)
                .lineLimit(1)
                .fixedSize()
        }
    }

    private func title(_ i: Int) -> String {
        let index = Int(control.min) + i
        if index >= 0, index < control.options.count, !control.options[index].isEmpty {
            return control.options[index].replacingOccurrences(of: "_", with: " ")
        }
        return String(index)
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
