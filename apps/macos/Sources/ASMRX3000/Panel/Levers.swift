import CAsmr
import SwiftUI

extension Control {
    /// ASMR_CONTROL_FLAG_LEVEL (1 << 4, flags::LEVEL in Rust): a group's loudness trim,
    /// "<group>.level", dB -40...+12, default 0, -40 mutes, NaN resets to 0 dB. It never follows
    /// its macro. Spelled out here so the shell builds against a core without the flag yet.
    static let levelFlag: UInt32 = 1 << 4
    var isLevel: Bool { flags & Control.levelFlag != 0 }
    /// A level at the bottom of its range is silent.
    var isMuted: Bool { isLevel && value <= min + 0.01 }
}

/// The engine's control tree in two plainly separate sections:
///
///   ЗВУК  how loud each group of sounds is: one dB trim per group (controls with
///         ASMR_CONTROL_FLAG_LEVEL), a mute on its pictogram, and the mix's clarity. Pure gain;
///         nothing in the forest changes.
///   МИР   what happens in the forest: the macro levers (river flow, wind speed, rain rate, the
///         hour, flight speed…). The slider's fill is the live waveform of that group, so the
///         lever shows what it does to the sound; its name and value are always written next to it.
///
/// A clip scene (underwater, space) has none of the forest's groups: its section СЦЕНА holds the
/// scene's own levers instead (motion, events, bed, creatures: ASMR_PARAM_SCENE_*), plus the
/// loudness of any physical sources it also has.
///
/// Every lever shows its name and value with units at rest. A world lever's pictogram (or
/// chevron) opens that group's drawer: its physical parameters, named, one group at a time.
/// Generic over the tree: groups added to the engine later land behind "ещё"; while the engine
/// has no level controls the sound section holds only the mix.
struct LeverPanel: View {
    let model: AppModel
    @ObservedObject var controls: ControlsModel
    @ObservedObject var audio: AudioEngine
    var stills: StillFrames?
    @AppStorage("levers.showAll") private var showAllStored = false
    @ObservedObject private var lang = Language.shared

    static let primary = [ASMR_GROUP_WATER, ASMR_GROUP_WIND, ASMR_GROUP_BIRDS, ASMR_GROUP_RAIN, ASMR_GROUP_ENVIRONMENT,
                         ASMR_GROUP_DIRECTION]
    /// Lives in the transport row (master) or in the sound section (mix) instead.
    static let elsewhere = [ASMR_GROUP_MASTER, ASMR_GROUP_MIX]
    /// Order of the groups in the sound section.
    static let soundOrder = [ASMR_GROUP_WATER, ASMR_GROUP_WIND, ASMR_GROUP_FOLIAGE, ASMR_GROUP_RAIN,
                             ASMR_GROUP_BIRDS, ASMR_GROUP_INSECTS, ASMR_GROUP_FIRE]

    init(model: AppModel, controls: ControlsModel, stills: StillFrames?) {
        self.model = model
        self.controls = controls
        audio = model.audio
        self.stills = stills
    }

    private var showAll: Bool { stills.map(\.showAll) ?? showAllStored }
    private var drawer: UInt32? { stills != nil ? stills?.drawer : controls.expanded }

    /// The world levers in display order: (always shown, behind "more").
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

    /// The clip scene's levers (AudioEngine.sceneControls), in their order.
    /// The scene's clip groups first (one lever each, level), then the four scene-wide ones.
    static func sceneLevers(_ macros: [Control]) -> [Control] {
        func rank(_ c: Control) -> UInt32 { c.id >= AudioEngine.clipControlBase ? c.id : 0x10_0000 + c.id }
        return macros.filter { $0.group >= AudioEngine.sceneGroup }.sorted { rank($0) < rank($1) }
    }

    /// The levers the lever section draws (the tracks behind their sliders follow them).
    static func levers(_ macros: [Control], scene: SceneInfo, flying: Bool, showAll: Bool) -> [Control] {
        scene.isClip ? sceneLevers(macros) : visible(macros, flying: flying, showAll: showAll)
    }

    static func layout(_ levers: [Control]) -> [ScopeRow] {
        levers.enumerated().map { i, m in
            ScopeRow(top: Float(i) * Float(Popover.leverHeight), height: Float(Popover.leverHeight),
                     track: Track(group: m.group), fraction: m.fraction)
        }
    }

    /// The groups' loudness trims, in the sound section's order. `ASMR_PREVIEW_LEVELS=1` makes
    /// stand-ins (inert, 0 dB) while the engine has none, to preview the layout.
    static func levels(_ controls: [Control]) -> [Control] {
        var found = controls.filter(\.isLevel)
        if found.isEmpty, ProcessInfo.processInfo.environment["ASMR_PREVIEW_LEVELS"] == "1" {
            found = controls.filter { $0.isMacro && soundOrder.contains(Int($0.group)) }.map { m in
                let head = m.key.split(separator: ".").first.map(String.init) ?? m.key
                let preview: [String: Float] = ["water": 0, "wind": -6, "foliage": -3, "rain": 0, "birds": 4, "insects": -40, "fire": 0]
                return Control(id: m.id + 50, group: m.group, parent: m.id, flags: Control.levelFlag, min: -40, max: 12,
                               value: preview[head] ?? 0, defaultValue: 0, key: head + ".level", unit: "dB")
            }
        }
        func rank(_ c: Control) -> Int { soundOrder.firstIndex(of: Int(c.group)) ?? 100 + Int(c.group) }
        return found.sorted { rank($0) < rank($1) }
    }

    var body: some View {
        let hour = controls.controls.first { $0.key == "env.time_of_day" }?.value
        if let id = drawer, let macro = controls.controls.first(where: { $0.id == id }) {
            Drawer(macro: macro, children: drawerChildren(of: id), hour: hour, scrolls: stills == nil,
                   status: Int(macro.group) == ASMR_GROUP_DIRECTION ? { Self.directionStatus(audio) } : nil,
                   set: set, close: { controls.expanded = nil })
        } else if audio.scene.isClip {
            VStack(alignment: .leading, spacing: 0) {
                sound(hour: hour, only: audio.scene.groups)
                clipScene(hour: hour)
            }
        } else {
            VStack(alignment: .leading, spacing: 0) {
                sound(hour: hour)
                world(hour: hour)
            }
        }
    }

    /// A group's parameters for its drawer: the level lives in the sound section, and a source
    /// gain is redundant once the group has a level.
    private func drawerChildren(of id: UInt32) -> [Control] {
        let all = controls.children(of: id)
        let hasLevel = all.contains(where: \.isLevel)
        return all.filter { c in
            !c.isLevel && !(hasLevel && c.key.hasSuffix(".gain"))
        }
    }

    /// The direction's status line: "thunderstorm · gust front — then: after the rain, ~6 min".
    static func directionStatus(_ audio: AudioEngine) -> String? {
        let d = audio.directionInfo()
        guard d.available != 0, d.active != 0 else { return nil }
        var now = ControlText.option("cue." + audio.directionEpisodeName(d.episode))
        let segment = audio.directionSegmentLabel(episode: d.episode, segment: d.segment)
        if !segment.isEmpty { now += " · " + ControlText.segment(segment) }
        let next = ControlText.option("cue." + audio.directionEpisodeName(d.next))
        let minutes = Swift.max(1, Int((d.episode_left_s / 60).rounded()))
        return now + " — " + tr("then: %@, ~%d min", next, minutes)
    }

    // MARK: Sound

    /// `only`: the groups the scene has sounds of (a clip scene); the mix is the forest's.
    @ViewBuilder private func sound(hour: Float?, only: Set<Int>? = nil) -> some View {
        let levels = Self.levels(controls.controls).filter { only?.contains(Int($0.group)) ?? true }
        let mix = only == nil ? controls.macros.first { Int($0.group) == ASMR_GROUP_MIX } : nil
        // Without level controls (an older core) the section keeps only the mix's clarity.
        if !levels.isEmpty || mix != nil {
            SectionHeader(title: tr("Sound"), note: levels.isEmpty ? tr("mix") : tr("group levels, dB"))
            let cells: [SoundCell.Kind] = levels.map { .level($0) } + (mix.map { [.mix($0)] } ?? [])
            let rows = stride(from: 0, to: cells.count, by: 2).map { Array(cells[$0..<Swift.min($0 + 2, cells.count)]) }
            VStack(spacing: 0) {
                ForEach(rows.indices, id: \.self) { r in
                    HStack(spacing: Popover.soundColumnGap) {
                        ForEach(rows[r], id: \.id) { cell in
                            SoundCell(kind: cell, hour: hour, meters: model.stage.meters, set: set,
                                      open: { controls.expanded = cell.id })
                        }
                        if rows[r].count == 1 { Spacer(minLength: 0) }
                    }
                    .frame(height: Popover.soundRowHeight)
                }
            }
            .padding(.bottom, Grid.x3)
        }
    }

    // MARK: World

    /// Russian name of the current flight landscape (flight.biome preset). In a journey the
    /// biome changes with the place: the frame says which one is under the listener
    /// ("путешествие · луга").
    static func biomeName(_ all: [Control], frame: AsmrFlightFrame? = nil) -> String {
        guard let c = all.first(where: { $0.key == "flight.biome" }) else { return tr("forest") }
        let i = Int(c.value.rounded())
        guard i >= 0 && i < c.options.count else { return tr("forest") }
        let name = ControlText.option(c.options[i])
        if c.options[i] == "journey", let f = frame, Int(f.biome) == i, Int(f.local_biome) < c.options.count, Int(f.local_biome) != i {
            return name + " · " + ControlText.option(c.options[Int(f.local_biome)])
        }
        return name
    }

    private func world(hour: Float?) -> some View {
        let split = Self.split(controls.macros, flying: audio.flying)
        let shown = showAll ? split.shown + split.more : split.shown
        return VStack(alignment: .leading, spacing: 0) {
            // Polled once a second: in a journey the biome under the listener changes as it flies.
            TimelineView(.periodic(from: .now, by: 1)) { _ in
                SectionHeader(title: tr("World"), note: audio.flying ? tr("flight: %@", Self.biomeName(controls.controls, frame: audio.readFlightFrame())) : tr("what happens in the forest"))
            }
            rows(shown, hour: hour)
            if !split.more.isEmpty {
                MoreRow(more: split.more, hour: hour, showAll: showAll) { showAllStored.toggle() }
                    .frame(height: Popover.leverHeight)
            }
        }
    }

    /// The lever rows over the live waveforms of their groups.
    private func rows(_ shown: [Control], hour: Float?) -> some View {
        ZStack(alignment: .topLeading) {
            Group {
                if let stills {
                    Image(decorative: stills.tracks, scale: 2).resizable()
                } else {
                    TracksSurface(stage: model.stage, rows: Self.layout(shown))
                }
            }
            .frame(width: Popover.sliderWidth, height: Popover.leverHeight * CGFloat(shown.count))
            .opacity(0.8)
            .padding(.leading, Popover.iconColumn + Popover.nameColumn)

            VStack(spacing: 0) {
                ForEach(shown, id: \.id) { m in
                    WorldRow(control: m, hour: hour, hasChildren: !drawerChildren(of: m.id).isEmpty,
                             meters: model.stage.meters, set: set, open: { controls.expanded = m.id })
                        .frame(height: Popover.leverHeight)
                }
            }
        }
    }

    // MARK: Clip scene

    private func clipScene(hour: Float?) -> some View {
        let grouped = !audio.clipGroups.isEmpty
        let plain: String = switch audio.scene.look {
        case .underwater: tr("what happens in the water")
        case .space: tr("what happens around you")
        default: tr("the scene's sounds")
        }
        let note = grouped ? tr("group levels; “all” moves every group") : plain
        return VStack(alignment: .leading, spacing: 0) {
            SectionHeader(title: tr("Scene"), note: note)
            rows(Self.sceneLevers(controls.macros), hour: hour)
        }
    }

    private func set(_ id: UInt32, _ value: Float) {
        model.stage.poke()
        model.audio.setControl(id, value)
        model.stage.refreshControls()
    }
}

/// "Звук ───── громкость групп, дБ": a section's name, a hairline, and what its levers are.
private struct SectionHeader: View {
    let title: String
    let note: String

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text(title)
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(Ink.paper)
                .fixedSize()
            Rectangle().fill(Ink.hairline).frame(height: 1)
            Text(note)
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(Ink.ash)
                .lineLimit(1)
        }
        .frame(height: Popover.headerHeight)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

// MARK: - Naming and values

enum ControlText {
    /// A parameter's name, in Russian: "water.rock_size" → "Размер камней". A macro is named by
    /// what it sets (see `lever`); a level by its group. Unknown keys fall back to the key itself.
    static func name(_ c: Control) -> String {
        if let n = clipName(c) { return n }
        if c.isLevel { return group(c) }
        if c.isMacro { return group(c) }
        if let n = names[c.key] { return tr(n) }
        let parts = c.key.split(separator: ".").map(String.init)
        if parts.count == 3, parts[1] == "focus" { return tr("Focus: %@", (groupNames[parts[2]].map(tr) ?? parts[2]).lowercased()) }
        let leaf = parts.last ?? c.key
        return sentence(leaf.replacingOccurrences(of: "_", with: " "))
    }

    /// The name of a world lever: what the macro is, physically.
    static func lever(_ c: Control) -> String {
        clipName(c) ?? names[c.key].map(tr) ?? group(c)
    }

    static func group(_ c: Control) -> String {
        if let t = clipTitle(c) { return t }
        switch Int(c.group) {
        case ASMR_GROUP_WATER: return tr("Water")
        case ASMR_GROUP_WIND: return tr("Wind")
        case ASMR_GROUP_FOLIAGE: return tr("Foliage")
        case ASMR_GROUP_RAIN: return tr("Rain")
        case ASMR_GROUP_BIRDS: return tr("Birds")
        case ASMR_GROUP_MASTER: return tr("Volume")
        case ASMR_GROUP_ENVIRONMENT: return tr("Time and weather")
        case ASMR_GROUP_INSECTS: return tr("Insects")
        case ASMR_GROUP_FIRE: return tr("Campfire")
        case ASMR_GROUP_FLIGHT: return tr("Flight")
        case ASMR_GROUP_MIX: return tr("Mix")
        case ASMR_GROUP_DIRECTION: return tr("Direction")
        default:
            let head = c.key.split(separator: ".").first.map(String.init) ?? c.key
            return groupNames[head].map(tr) ?? sentence(head.replacingOccurrences(of: "_", with: " "))
        }
    }

    private static let groupNames = [
        "water": N_("Water"), "wind": N_("Wind"), "foliage": N_("Foliage"), "rain": N_("Rain"), "birds": N_("Birds"),
        "insects": N_("Insects"), "fire": N_("Campfire"), "flight": N_("Flight"), "mix": N_("Mix"), "env": N_("Time and weather"),
        "scene": N_("Scene"),
    ]

    private static let names = [
        // Macros: the world levers.
        "water.flow": N_("River flow"), "wind.speed": N_("Wind speed"), "foliage.density": N_("Foliage density"),
        "rain.rate": N_("Rain intensity"), "birds.activity": N_("Bird activity"), "env.time_of_day": N_("Time of day"),
        "insects.activity": N_("Insect activity"), "fire.heat": N_("Fire heat"), "flight.speed": N_("Flight speed"),
        "mix.clarity": N_("Clarity"),
        // Clip scenes (AudioEngine.SceneMacro).
        "scene.motion": N_("All: motion"), "scene.events": N_("All: events"), "scene.bed": N_("All: background"), "scene.creatures": N_("All: creatures"),
        // Water.
        "water.regime": N_("Water body"), "water.speed": N_("Current speed"), "water.depth": N_("Depth"),
        "water.width": N_("Width"), "water.rock_size": N_("Rock size"), "water.aeration": N_("Foam and bubbles"),
        "water.turbulence": N_("Turbulence"), "water.gain": N_("Water volume"),
        // Wind and foliage.
        "wind.gustiness": N_("Gustiness"), "wind.exposure": N_("Openness"), "wind.character": N_("Character"),
        "wind.whistle": N_("Whistle"), "foliage.dryness": N_("Dryness"), "foliage.flutter": N_("Flutter"),
        "foliage.creak": N_("Trunk creak"), "foliage.exposure": N_("Openness"),
        // Rain.
        "rain.storm": N_("Thunderstorm"), "rain.surface": N_("Surface"), "rain.drying": N_("Drying"), "rain.gain": N_("Rain volume"),
        // Birds, environment, insects, fire.
        "birds.orbit": N_("Circling close"), "env.temperature": N_("Temperature"), "env.humidity": N_("Humidity"),
        "env.season": N_("Season"), "env.wind_direction": N_("Wind from"), "insects.warmth": N_("Warmth"), "fire.moisture": N_("Wood dampness"), "fire.resin": N_("Resin"),
        // Flight.
        "flight.altitude": N_("Altitude"), "flight.curviness": N_("Winding"), "flight.biome": N_("Landscape"),
        "flight.density": N_("Forest density"), "flight.seed": N_("Terrain"), "flight.airflow": N_("Air noise"),
        "flight.steer_by_head": N_("Steer with head"), "flight.turn_rate": N_("Turn rate"),
        // Mix.
        "mix.density": N_("Birds at once"), "mix.soloists": N_("Soloists"), "mix.loudness": N_("Target loudness"),
        "mix.dynamics": N_("Leveling"),
        // Direction.
        "direction.amount": N_("Direction"), "direction.weather": N_("Fair ↔ wet"), "direction.energy": N_("Still ↔ lively"),
        "direction.change": N_("Steady ↔ eventful"), "direction.clock": N_("Time speed"), "direction.hold": N_("Hold my changes"),
        "direction.cue": N_("Call up"),
    ]

    private static let options = [
        "scene": N_("as in the scene"),
        // water.regime
        "still_lake": N_("still lake"), "lake_lapping": N_("lapping lake"), "slow_river": N_("slow river"),
        "brook_pebbles": N_("brook over pebbles"), "brook_rocks": N_("brook over rocks"), "rapids": N_("rapids"),
        "small_waterfall": N_("small waterfall"), "drips": N_("drips"), "trickle": N_("trickle"),
        // wind.character
        "breeze": N_("breeze"), "forest_roar": N_("forest roar"), "storm": N_("storm"), "power_lines": N_("power lines"),
        "fence_wire": N_("fence wire"), "gap_howl": N_("howl in a gap"), "hollow_tree": N_("hollow tree"), "whistle": N_("whistle"),
        // rain.surface
        "light_drizzle": N_("drizzle"), "steady_rain": N_("steady rain"), "downpour": N_("downpour"), "on_leaves": N_("on leaves"),
        "on_soil": N_("on soil"), "on_rock": N_("on rock"), "on_water": N_("on water"), "puddles": N_("puddles"),
        "tin_roof": N_("tin roof"), "wood_deck": N_("wood deck"), "meadow": N_("meadow"), "under_canopy": N_("under the canopy"),
        "under_conifer": N_("under a spruce"),
        // env.season
        "spring": N_("spring"), "summer": N_("summer"), "autumn": N_("autumn"), "winter": N_("winter"),
        // flight.biome
        "mixed": N_("mixed forest"), "broadleaf": N_("oak forest"), "taiga": N_("taiga"), "river_valley": N_("river valley"),
        "meadows": N_("meadows"), "highlands": N_("highlands"), "desert": N_("desert"), "snowy_mountains": N_("snowy mountains"),
        "lake_shore": N_("lake shore"), "sea_coast": N_("sea coast"), "big_river": N_("big river"),
        "journey": N_("journey"),
        // flight.steer_by_head
        "off": N_("no"), "yaw": N_("by turning"), "yaw_pitch": N_("by turning and tilting"),
        // direction.cue and the spells (AudioEngine prefixes them "cue." so "steady_rain" keeps both names).
        "cue.none": N_("—"), "cue.next": N_("next"), "cue.clear": N_("clear"), "cue.calm": N_("calm"),
        "cue.breeze": N_("breeze"), "cue.windy": N_("windy"), "cue.overcast": N_("overcast"), "cue.drizzle": N_("drizzle"),
        "cue.shower": N_("shower"), "cue.steady_rain": N_("long rain"), "cue.thunderstorm": N_("thunderstorm"),
        "cue.after_rain": N_("after the rain"), "cue.mist": N_("mist"),
    ]

    /// Stretches of a spell (direction/episodes.json labels).
    private static let segments = [
        "clear": N_("clear"), "still air": N_("still air"), "breeze": N_("breeze"), "wind rising": N_("wind rising"),
        "windy": N_("windy"), "wind easing": N_("wind easing"), "clouding over": N_("clouding over"), "drizzle": N_("drizzle"),
        "darkening": N_("darkening"), "gust front": N_("gust front"), "downpour": N_("downpour"), "tapering off": N_("tapering off"),
        "rain setting in": N_("rain setting in"), "steady rain": N_("steady rain"), "rain easing": N_("rain easing"),
        "distant thunder": N_("distant thunder"), "calm before the storm": N_("calm before the storm"),
        "the cell overhead": N_("the cell overhead"), "trailing rain": N_("trailing rain"),
        "storm moving away": N_("storm moving away"), "after the rain": N_("after the rain"), "mist": N_("mist"),
    ]

    static func segment(_ raw: String) -> String { segments[raw].map(tr) ?? raw }

    static func option(_ raw: String) -> String {
        options[raw].map(tr) ?? raw.replacingOccurrences(of: "_", with: " ")
    }

    private static let units = [
        "m/s": N_("m/s"), "m": N_("m"), "mm/h": N_("mm/h"), "kW": N_("kW"), "°C": N_("°C"), "°/s": N_("°/s"), "x": N_("x"), "min": N_("min"),
        "dB": N_("dB"), "LUFS": N_("LUFS"), "Hz": N_("Hz"),
    ]

    // MARK: Clip groups (AudioEngine.clipGroups)

    /// Group titles and icon keys from the scene (`controls` block), set when a core is made.
    static var clipTitles: [String: String] { ClipGroupLabels.titles }
    static var clipIcons: [String: String] { ClipGroupLabels.icons }

    private static let clipFallbackTitles = [
        "current": N_("Current"), "whales": N_("Whales"), "dolphins": N_("Dolphins"), "bubbles": N_("Bubbles"),
        "plasma": N_("Plasma"), "flybys": N_("Flybys"), "beams": N_("Beams"), "orbits": N_("Orbits"),
    ]
    private static let clipFallbackIcons = [
        "current": "water", "whales": "whale", "dolphins": "dolphin", "bubbles": "bubbles",
        "plasma": "plasma", "flybys": "flyby", "beams": "beam", "orbits": "orbit",
    ]
    private static let clipLeaves = [
        "level": N_("Volume"), "events": N_("Events"), "motion": N_("Motion"), "distance": N_("Distance"),
        "pitch": N_("Pitch"), "flow": N_("Current speed"), "turbulence": N_("Turbulence"),
        "hiss": N_("Hiss center"), "whistlers": N_("Whistlers"),
    ]

    /// ("whales", "events") from "clip.whales.events".
    private static func clipParts(_ c: Control) -> (id: String, leaf: String)? {
        guard c.key.hasPrefix("clip.") else { return nil }
        let p = c.key.split(separator: ".").map(String.init)
        return p.count == 3 ? (p[1], p[2]) : nil
    }

    static func clipTitle(_ c: Control) -> String? {
        guard let p = clipParts(c) else { return nil }
        return clipTitles[p.id].flatMap { $0.isEmpty ? nil : tr($0) } ?? clipFallbackTitles[p.id].map(tr) ?? sentence(p.id)
    }

    /// A group's lever is named by the group; its aspects by what they change.
    static func clipName(_ c: Control) -> String? {
        guard let p = clipParts(c) else { return nil }
        return c.isMacro ? clipTitle(c) : clipLeaves[p.leaf].map(tr) ?? sentence(p.leaf)
    }

    static func clipIcon(_ c: Control) -> Icon? {
        guard let p = clipParts(c) else { return nil }
        if c.isMacro {
            let key = clipIcons[p.id].flatMap { $0.isEmpty ? nil : $0 } ?? clipFallbackIcons[p.id] ?? ""
            switch key {
            case "water", "current": return .water
            case "whale": return .whale
            case "dolphin": return .dolphin
            case "bubbles": return .bubbles
            case "plasma": return .plasma
            case "flyby", "comet": return .comet
            case "beam": return .beam
            case "orbit": return .orbit
            case "wind": return .wind
            case "tone": return .tone
            default: return .volume
            }
        }
        switch p.leaf {
        case "level": return .gain
        case "events": return .dot
        case "motion": return .speed
        case "distance": return .width
        case "pitch", "hiss", "whistlers": return .tone
        case "flow": return .flow
        case "turbulence": return .gust
        default: return .dot
        }
    }

    static func sentence(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    private static func minus(_ s: String) -> String { s.replacingOccurrences(of: "-", with: "−") }

    /// "+3 dB" / "−12 dB".
    private static func decibels(_ db: Int) -> String {
        db > 0 ? tr("+%d dB", db) : minus(tr("%d dB", db))
    }

    static func value(_ c: Control) -> String {
        let index = Int(c.value.rounded())
        if c.isDiscrete {
            if index >= 0, index < c.options.count, !c.options[index].isEmpty {
                return option(c.options[index])
            }
            return String(index)
        }
        if c.isLevel {
            if c.isMuted { return tr("off") }
            return decibels(Int(c.value.rounded()))
        }
        if let p = clipParts(c) {
            switch p.leaf {
            case "level":
                if c.value <= c.min + 0.01 { return tr("off") }
                return decibels(Int(c.value.rounded()))
            case "events" where c.value < 0.01: return tr("off")
            case "motion" where c.value < 0.01: return tr("stop")
            case "whistlers": return tr("+%d /min", Int(c.value.rounded()))
            default: break
            }
        }
        switch c.key {
        case "foliage.dryness" where c.value < 0: return tr("by season")
        case "scene.motion" where c.value < 0.01: return tr("stop")
        case "scene.events" where c.value < 0.01, "scene.bed" where c.value < 0.01, "scene.creatures" where c.value < 0.01:
            return tr("off")
        case "flight.seed": return tr("#%d", index)
        case "mix.soloists": return String(index)
        default: break
        }
        if c.unit == "h" {
            let m = Int((c.value * 60).rounded()) % (24 * 60)
            return String(format: "%02d:%02d", m / 60, m % 60)
        }
        if c.unit == "x" {
            return "×" + (c.value >= 10 ? String(format: "%.0f", locale: L10n.locale, c.value) : String(format: "%.1f", locale: L10n.locale, c.value))
        }
        if c.unit.isEmpty {
            // Unitless: a share of the range, or a factor when 1 is "as built" in the middle.
            if c.min < 0 { return minus((c.value > 0 ? "+" : "") + tr("%d%%", Int((c.value / Swift.max(c.max, 1) * 100).rounded()))) }
            if c.max > 1.01 { return "×" + String(format: "%.1f", locale: L10n.locale, c.value) }
            return tr("%d%%", Int((c.fraction * 100).rounded()))
        }
        let a = abs(c.value)
        let loc = L10n.locale
        let v = a < 0.005 ? "0" : a >= 10 ? String(format: "%.0f", locale: loc, c.value)
            : a >= 1 ? String(format: "%.1f", locale: loc, c.value) : String(format: "%.2f", locale: loc, c.value)
        return minus("\(v) \(units[c.unit].map(tr) ?? c.unit)")
    }
}

// MARK: - Sound

/// One cell of the sound section: the group's pictogram (click: mute), its name, a dB rail
/// with a mark at 0 dB, and the value. The mix's clarity takes the last cell and opens its
/// drawer instead.
private struct SoundCell: View {
    @ObservedObject private var lang = Language.shared
    enum Kind {
        case level(Control)
        case mix(Control)
        var control: Control { switch self { case let .level(c), let .mix(c): return c } }
        var id: UInt32 { control.id }
    }

    let kind: Kind
    var hour: Float?
    @ObservedObject var meters: Meters
    let set: (UInt32, Float) -> Void
    let open: () -> Void
    @State private var hovering = false
    @State private var dragging = false
    /// What a mute toggles back to.
    @State private var unmuted: Float = 0

    var body: some View {
        let c = kind.control
        let muted = c.isMuted
        let track = Track(group: c.group)
        let audible = track.map { meters.audible.contains($0) } ?? false
        HStack(spacing: 0) {
            Button(action: tapIcon) {
                IconView(icon: Icon.forControl(c, hour: hour),
                         color: muted ? Ink.ash : hovering ? Ink.paper : audible ? Ink.paper.opacity(0.85) : Ink.graphite,
                         size: Popover.smallIconSize, animating: hovering && meters.live && !muted, struck: muted, lineWidth: 1)
                    .frame(width: Popover.iconColumn, height: Popover.soundRowHeight, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .help(isMix ? tr("Mix: details") : muted ? tr("unmute %@", ControlText.group(c).lowercased()) : tr("mute %@", ControlText.group(c).lowercased()))
            .accessibilityLabel(isMix ? tr("Mix: details") : muted ? tr("Unmute: %@", ControlText.group(c)) : tr("Mute: %@", ControlText.group(c)))

            Text(isMix ? ControlText.lever(c) : ControlText.group(c))
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(muted ? Ink.ash : hovering || dragging ? Ink.paper : Ink.graphite)
                .lineLimit(1)
                .frame(width: Popover.soundNameColumn, alignment: .leading)

            GeometryReader { geo in
                Rail(control: c, width: geo.size.width, active: hovering || dragging, muted: muted)
                    .gesture(DragGesture(minimumDistance: 0)
                        .onChanged { g in
                            dragging = true
                            set(c.id, c.value(atFraction: Float(g.location.x / geo.size.width)))
                        }
                        .onEnded { _ in dragging = false })
                    .simultaneousGesture(TapGesture(count: 2).onEnded { set(c.id, isMix ? c.defaultValue : .nan) })
            }

            // Value, then a chevron slot in every cell, so both columns' values line up whether
            // or not the cell opens a drawer.
            if isMix {
                Button(action: open) {
                    HStack(spacing: 0) {
                        Text(ControlText.value(c))
                            .foregroundStyle(hovering ? Ink.paper : Ink.paper.opacity(0.85))
                            .frame(width: Popover.soundValueColumn, alignment: .trailing)
                        Chevron(lit: hovering)
                    }
                    .frame(height: Popover.soundRowHeight)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
            } else {
                Text(ControlText.value(c))
                    .foregroundStyle(muted ? Ink.ash : c.value == 0 ? Ink.graphite : Ink.paper)
                    .frame(width: Popover.soundValueColumn, alignment: .trailing)
                    .padding(.trailing, Popover.chevronColumn)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(1)
        .frame(width: Popover.soundColumnWidth)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .help(isMix ? tr("How spread apart the sounds are: double-click to reset") : tr("Double-click the rail: 0 dB"))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(isMix ? ControlText.lever(c) : tr("Volume: %@", ControlText.group(c)))
        .accessibilityValue(ControlText.value(c))
        .accessibilityAdjustableAction { dir in
            let step: Float = isMix ? 0.05 : 1 / 52
            set(c.id, c.value(atFraction: c.fraction + (dir == .increment ? step : -step)))
        }
    }

    private var isMix: Bool { if case .mix = kind { return true } else { return false } }

    private func tapIcon() {
        let c = kind.control
        if isMix { open(); return }
        if c.isMuted {
            set(c.id, unmuted > c.min + 0.01 ? unmuted : 0)
        } else {
            unmuted = c.value
            set(c.id, c.min)
        }
    }
}

/// A thin rail: filled up to the knob, a tick at the default (0 dB).
private struct Rail: View {
    let control: Control
    let width: CGFloat
    let active: Bool
    var muted = false

    var body: some View {
        let x = CGFloat(control.fraction) * width
        let zero = control.max > control.min ? CGFloat((control.defaultValue - control.min) / (control.max - control.min)) * width : 0
        Canvas { ctx, size in
            let mid = size.height / 2
            SliderInk.rail(ctx, width: size.width, mid: mid, fill: x, fillColor: muted ? Ink.hairline : Ink.graphite)
            if control.isLevel {
                // The 0 dB mark, as tall as the resting knob is wide apart: a quiet tick.
                ctx.fill(Path(CGRect(x: min(max(zero - 0.5, 0), size.width - 1), y: mid - Grid.x1 - 1, width: 1, height: Grid.x2 + 2)),
                         with: .color(Ink.ash))
            }
            SliderInk.knob(ctx, at: x, width: size.width, mid: mid, active: active, color: muted ? Ink.ash : Ink.paper)
        }
        .contentShape(Rectangle())
    }
}

// MARK: - World

private struct WorldRow: View {
    @ObservedObject private var lang = Language.shared
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
        let lit = hovering || dragging
        HStack(spacing: 0) {
            Button(action: open) {
                HStack(spacing: 0) {
                    IconView(icon: Icon.forControl(control, hour: hour),
                             color: lit ? Ink.paper : audible ? Ink.paper.opacity(0.85) : Ink.graphite,
                             size: Popover.iconSize, level: audible ? 0.6 : 0, animating: hovering && meters.live)
                        .frame(width: Popover.iconColumn, alignment: .leading)
                    Text(ControlText.lever(control))
                        .font(Ink.text(Ink.Size.body))
                        .foregroundStyle(lit ? Ink.paper : Ink.graphite)
                        .lineLimit(1)
                        .frame(width: Popover.nameColumn - Grid.x2, alignment: .leading)
                }
                .frame(height: Popover.leverHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .disabled(!hasChildren)
            .padding(.trailing, Grid.x2)
            .help(hasChildren ? tr("%@: details", ControlText.group(control)) : ControlText.group(control))
            .accessibilityLabel(hasChildren ? tr("%@: details", ControlText.group(control)) : ControlText.group(control))

            GeometryReader { geo in
                let w = geo.size.width
                let x = CGFloat(control.fraction) * w
                let knob = lit ? Grid.knobActive : Grid.knob
                ZStack(alignment: .topLeading) {
                    Color.clear.contentShape(Rectangle())
                    RoundedRectangle(cornerRadius: 1)
                        .fill(Ink.paper)
                        .frame(width: knob.width, height: knob.height)
                        .position(x: min(max(x, knob.width / 2), w - knob.width / 2), y: geo.size.height / 2)
                }
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { g in
                        dragging = true
                        set(control.id, control.value(atFraction: Float(g.location.x / w)))
                    }
                    .onEnded { _ in dragging = false })
            }
            .frame(width: Popover.sliderWidth)
            .accessibilityElement()
            .accessibilityLabel(ControlText.lever(control))
            .accessibilityValue(ControlText.value(control))
            .accessibilityAdjustableAction { dir in
                set(control.id, control.value(atFraction: control.fraction + (dir == .increment ? 0.05 : -0.05)))
            }

            Text(ControlText.value(control))
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(lit ? Ink.paper : Ink.paper.opacity(0.8))
                .lineLimit(1)
                .frame(width: Popover.valueColumn, alignment: .trailing)

            Button(action: open) {
                Chevron(lit: lit)
                    .frame(height: Popover.leverHeight)
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

/// "Ещё": the hidden groups' pictograms; click to show their levers too.
private struct MoreRow: View {
    @ObservedObject private var lang = Language.shared
    let more: [Control]
    var hour: Float?
    let showAll: Bool
    let toggle: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: Grid.x2) {
                IconView(icon: showAll ? .back : .more, color: hovering ? Ink.paper : Ink.graphite, size: Popover.smallIconSize,
                         lineWidth: 1)
                    .rotationEffect(.degrees(showAll ? 90 : 0))
                    .frame(width: Popover.iconColumn - Grid.x2, alignment: .leading)
                if showAll {
                    Text(tr("Collapse")).foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                } else {
                    Text(tr("More %d:", more.count)).foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                    ForEach(more, id: \.id) { m in
                        HStack(spacing: Grid.x1) {
                            IconView(icon: Icon.forControl(m, hour: hour), color: hovering ? Ink.graphite : Ink.ash, size: 14,
                                     lineWidth: 1)
                            Text(ControlText.group(m).lowercased()).foregroundStyle(hovering ? Ink.graphite : Ink.ash)
                        }
                    }
                }
                Spacer()
            }
            .font(Ink.text(Ink.Size.caption))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering = $0 }
        .accessibilityLabel(showAll ? tr("Show less") : tr("Show %d more", more.count))
    }
}

// MARK: - Drawer

/// One group's physical parameters, named, with their values. Replaces the levers until closed.
private struct Drawer: View {
    @ObservedObject private var lang = Language.shared
    let macro: Control
    let children: [Control]
    var hour: Float?
    let scrolls: Bool
    var status: (() -> String?)?
    let set: (UInt32, Float) -> Void
    let close: () -> Void
    @State private var hoveringBack = false

    /// Nine and a half rows: the half row says there is more to scroll.
    static let maxHeight: CGFloat = Popover.childHeight * 9.5

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Same height and hairline as a section header, so the drawer reads as the section
            // it replaced: "‹ Вода ───── Течение воды  60 %".
            HStack(spacing: Grid.x2) {
                Button(action: close) {
                    HStack(spacing: Grid.x1) {
                        IconView(icon: .back, color: hoveringBack ? Ink.paper : Ink.graphite, size: 14, lineWidth: 1)
                        Text(ControlText.group(macro))
                            .font(Ink.text(Ink.Size.body))
                            .foregroundStyle(Ink.paper)
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
                .accessibilityLabel(tr("Back to all levers"))
                .help(tr("Back   ⌘←"))
                Rectangle().fill(Ink.hairline).frame(height: 1)
                Text("\(ControlText.lever(macro))  \(ControlText.value(macro))")
                    .font(Ink.text(Ink.Size.caption))
                    .foregroundStyle(Ink.ash)
                    .lineLimit(1)
                    .fixedSize()
            }
            .frame(height: Popover.headerHeight)

            if let status {
                // Polled once a second, like the flight note: the spell moves on by itself.
                TimelineView(.periodic(from: .now, by: 1)) { _ in
                    Text(status() ?? "")
                        .font(Ink.text(Ink.Size.caption))
                        .foregroundStyle(Ink.ash)
                        .lineLimit(1)
                        .padding(.leading, Popover.iconColumn)
                        .frame(maxWidth: .infinity, minHeight: Popover.childHeight / 2, alignment: .leading)
                }
            }

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
    @ObservedObject private var lang = Language.shared
    let control: Control
    var hour: Float?
    let set: (UInt32, Float) -> Void
    @State private var hovering = false

    var body: some View {
        // The world levers' columns: pictogram, name, slider, then the value across the value
        // and chevron columns (a parameter has no drawer), flush with the gutter.
        HStack(spacing: 0) {
            IconView(icon: Icon.forControl(control, hour: hour), color: hovering ? Ink.paper : Ink.graphite,
                     size: Popover.smallIconSize, lineWidth: 1)
                .frame(width: Popover.iconColumn, alignment: .leading)
            Text(ControlText.name(control))
                .font(Ink.text(Ink.Size.body))
                .foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(width: Popover.nameColumn - Grid.x2, alignment: .leading)
                .padding(.trailing, Grid.x2)
            if control.isDiscrete {
                Presets(control: control, set: set)
            } else {
                GeometryReader { geo in
                    FineSlider(control: control, width: geo.size.width, active: hovering, set: set)
                }
                .frame(width: Popover.sliderWidth)
                Text(ControlText.value(control))
                    .font(Ink.text(Ink.Size.body))
                    .foregroundStyle(control.isOverridden ? Ink.paper : Ink.graphite)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .help(control.isOverridden ? tr("Double-click the rail: follow the “%@” lever again", ControlText.lever(control)) : "")
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
            SliderInk.rail(ctx, width: size.width, mid: mid, fill: x,
                           fillColor: control.isOverridden ? Ink.paper.opacity(0.85) : Ink.graphite)
            SliderInk.knob(ctx, at: x, width: size.width, mid: mid, active: active, color: Ink.paper)
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
    @ObservedObject private var lang = Language.shared
    let control: Control
    let set: (UInt32, Float) -> Void
    @State private var hovered: Int?

    var body: some View {
        let n = max(Int((control.max - control.min).rounded()) + 1, 2)
        let chosen = Int((control.value - control.min).rounded())
        HStack(spacing: 0) {
            ForEach(0..<n, id: \.self) { i in
                Button { set(control.id, control.min + Float(i)) } label: {
                    RoundedRectangle(cornerRadius: 1)
                        .fill(i == chosen ? Ink.paper : hovered == i ? Ink.graphite : Ink.ash.opacity(0.6))
                        .frame(width: i == chosen ? 6 : 4, height: i == chosen ? 6 : 4)
                        .frame(width: Grid.x4, height: Popover.childHeight)
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
            return ControlText.option(control.options[index])
        }
        return String(index)
    }
}

/// The "opens a drawer" mark at the end of a row: one size, one weight, everywhere.
private struct Chevron: View {
    var lit = false

    var body: some View {
        IconView(icon: .chevron, color: lit ? Ink.paper : Ink.ash, size: 14, lineWidth: 1)
            .frame(width: Popover.chevronColumn, alignment: .trailing)
    }
}

/// The rail and knob every slider in the panel draws (sound levels, drawer parameters; the
/// world levers draw their rail in the tracks shader with the same values).
enum SliderInk {
    static func rail(_ ctx: GraphicsContext, width: CGFloat, mid: CGFloat, fill: CGFloat, fillColor: Color) {
        let h = Grid.rail
        ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - h / 2, width: width, height: h), cornerRadius: h / 2), with: .color(Ink.hairline))
        ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - h / 2, width: fill, height: h), cornerRadius: h / 2), with: .color(fillColor))
    }

    static func knob(_ ctx: GraphicsContext, at x: CGFloat, width: CGFloat, mid: CGFloat, active: Bool, color: Color) {
        let k = active ? Grid.knobActive : Grid.knob
        ctx.fill(Path(roundedRect: CGRect(x: min(max(x - k.width / 2, 0), width - k.width), y: mid - k.height / 2,
                                          width: k.width, height: k.height), cornerRadius: 1), with: .color(color))
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
        // Clip scenes: beds play as water sources, creatures as insects (clips.rs, ClipClass::kinds).
        case Int(AudioEngine.sceneGroup) + AudioEngine.SceneMacro.bed.rawValue: self = .water
        case Int(AudioEngine.sceneGroup) + AudioEngine.SceneMacro.creatures.rawValue: self = .insects
        // A clip group (AudioEngine.clipGroupNumber): metered on the track its sounds play as.
        case 200..<400:
            guard let t = Track(rawValue: (Int(group) - 200) / 16) else { return nil }
            self = t
        default: return nil   // environment shapes every sound; it has no waveform of its own
        }
    }
}
