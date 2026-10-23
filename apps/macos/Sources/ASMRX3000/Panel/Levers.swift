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

    static let primary = [ASMR_GROUP_WATER, ASMR_GROUP_WIND, ASMR_GROUP_BIRDS, ASMR_GROUP_RAIN, ASMR_GROUP_ENVIRONMENT]
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

    // MARK: Sound

    /// `only`: the groups the scene has sounds of (a clip scene); the mix is the forest's.
    @ViewBuilder private func sound(hour: Float?, only: Set<Int>? = nil) -> some View {
        let levels = Self.levels(controls.controls).filter { only?.contains(Int($0.group)) ?? true }
        let mix = only == nil ? controls.macros.first { Int($0.group) == ASMR_GROUP_MIX } : nil
        // Without level controls (an older core) the section keeps only the mix's clarity.
        if !levels.isEmpty || mix != nil {
            SectionHeader(title: "Звук", note: levels.isEmpty ? "микс" : "громкость групп, дБ")
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
        guard let c = all.first(where: { $0.key == "flight.biome" }) else { return "лес" }
        let i = Int(c.value.rounded())
        guard i >= 0 && i < c.options.count else { return "лес" }
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
                SectionHeader(title: "Мир", note: audio.flying ? "полёт: " + Self.biomeName(controls.controls, frame: audio.readFlightFrame()) : "что происходит в лесу")
            }
            rows(shown, hour: hour)
            if !split.more.isEmpty {
                MoreRow(more: split.more, hour: hour, showAll: showAll) { showAllStored.toggle() }
                    .frame(height: 24)
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
        case .underwater: "что происходит в воде"
        case .space: "что происходит вокруг"
        default: "звуки сцены"
        }
        let note = grouped ? "громкость групп; «всё» — на всех сразу" : plain
        return VStack(alignment: .leading, spacing: 0) {
            SectionHeader(title: "Сцена", note: note)
            rows(Self.sceneLevers(controls.macros), hour: hour)
        }
    }

    private func set(_ id: UInt32, _ value: Float) {
        model.stage.poke()
        model.audio.setControl(id, value)
        model.stage.refreshControls()
    }
}

/// "ЗВУК ───── громкость групп, дБ": a section's name, a hairline, and what its levers are.
private struct SectionHeader: View {
    let title: String
    let note: String

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text(title.uppercased())
                .font(Ink.text(Ink.Size.caption))
                .tracking(1.2)
                .foregroundStyle(Ink.paper.opacity(0.8))
                .fixedSize()
            Rectangle().fill(Ink.hairline).frame(height: 1)
            Text(note)
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(Ink.ash)
                .fixedSize()
        }
        .frame(height: 18)
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
        if let n = names[c.key] { return n }
        let parts = c.key.split(separator: ".").map(String.init)
        if parts.count == 3, parts[1] == "focus" { return "Фокус: " + (groupNames[parts[2]] ?? parts[2]).lowercased() }
        let leaf = parts.last ?? c.key
        return sentence(leaf.replacingOccurrences(of: "_", with: " "))
    }

    /// The name of a world lever: what the macro is, physically.
    static func lever(_ c: Control) -> String {
        clipName(c) ?? names[c.key] ?? group(c)
    }

    static func group(_ c: Control) -> String {
        if let t = clipTitle(c) { return t }
        switch Int(c.group) {
        case ASMR_GROUP_WATER: return "Вода"
        case ASMR_GROUP_WIND: return "Ветер"
        case ASMR_GROUP_FOLIAGE: return "Листва"
        case ASMR_GROUP_RAIN: return "Дождь"
        case ASMR_GROUP_BIRDS: return "Птицы"
        case ASMR_GROUP_MASTER: return "Громкость"
        case ASMR_GROUP_ENVIRONMENT: return "Время и погода"
        case ASMR_GROUP_INSECTS: return "Насекомые"
        case ASMR_GROUP_FIRE: return "Костёр"
        case ASMR_GROUP_FLIGHT: return "Полёт"
        case ASMR_GROUP_MIX: return "Микс"
        default:
            let head = c.key.split(separator: ".").first.map(String.init) ?? c.key
            return groupNames[head] ?? sentence(head.replacingOccurrences(of: "_", with: " "))
        }
    }

    private static let groupNames = [
        "water": "Вода", "wind": "Ветер", "foliage": "Листва", "rain": "Дождь", "birds": "Птицы",
        "insects": "Насекомые", "fire": "Костёр", "flight": "Полёт", "mix": "Микс", "env": "Время и погода",
        "scene": "Сцена",
    ]

    private static let names = [
        // Macros: the world levers.
        "water.flow": "Течение воды", "wind.speed": "Скорость ветра", "foliage.density": "Густота листвы",
        "rain.rate": "Сила дождя", "birds.activity": "Активность птиц", "env.time_of_day": "Время суток",
        "insects.activity": "Активность насекомых", "fire.heat": "Жар костра", "flight.speed": "Скорость полёта",
        "mix.clarity": "Ясность",
        // Clip scenes (AudioEngine.SceneMacro).
        "scene.motion": "Всё: движение", "scene.events": "Всё: события", "scene.bed": "Всё: фон", "scene.creatures": "Всё: существа",
        // Water.
        "water.regime": "Водоём", "water.speed": "Скорость течения", "water.depth": "Глубина",
        "water.width": "Ширина", "water.rock_size": "Размер камней", "water.aeration": "Пена и пузыри",
        "water.turbulence": "Турбулентность", "water.gain": "Громкость воды",
        // Wind and foliage.
        "wind.gustiness": "Порывистость", "wind.exposure": "Открытость", "wind.character": "Характер",
        "wind.whistle": "Свист", "foliage.dryness": "Сухость", "foliage.flutter": "Трепет",
        "foliage.creak": "Скрип стволов", "foliage.exposure": "Открытость",
        // Rain.
        "rain.storm": "Гроза", "rain.surface": "Поверхность", "rain.drying": "Высыхание", "rain.gain": "Громкость дождя",
        // Birds, environment, insects, fire.
        "birds.orbit": "Кружат рядом", "env.temperature": "Температура", "env.humidity": "Влажность",
        "env.season": "Сезон", "env.wind_direction": "Ветер откуда", "insects.warmth": "Тепло", "fire.moisture": "Влажность дров", "fire.resin": "Смола",
        // Flight.
        "flight.altitude": "Высота", "flight.curviness": "Извилистость", "flight.biome": "Ландшафт",
        "flight.density": "Густота леса", "flight.seed": "Местность", "flight.airflow": "Шум воздуха",
        "flight.steer_by_head": "Рулить головой", "flight.turn_rate": "Скорость поворота",
        // Mix.
        "mix.density": "Птиц одновременно", "mix.soloists": "Солистов", "mix.loudness": "Целевая громкость",
        "mix.dynamics": "Выравнивание",
    ]

    private static let options = [
        "scene": "как в сцене",
        // water.regime
        "still_lake": "тихое озеро", "lake_lapping": "плеск озера", "slow_river": "медленная река",
        "brook_pebbles": "ручей по гальке", "brook_rocks": "ручей по камням", "rapids": "пороги",
        "small_waterfall": "водопадик", "drips": "капель", "trickle": "струйка",
        // wind.character
        "breeze": "бриз", "forest_roar": "гул леса", "storm": "буря", "power_lines": "провода",
        "fence_wire": "проволока", "gap_howl": "вой в щели", "hollow_tree": "дупло", "whistle": "свист",
        // rain.surface
        "light_drizzle": "морось", "steady_rain": "ровный дождь", "downpour": "ливень", "on_leaves": "по листьям",
        "on_soil": "по земле", "on_rock": "по камню", "on_water": "по воде", "puddles": "лужи",
        "tin_roof": "жесть", "wood_deck": "доски", "meadow": "луг", "under_canopy": "под кроной",
        "under_conifer": "под елью",
        // env.season
        "spring": "весна", "summer": "лето", "autumn": "осень", "winter": "зима",
        // flight.biome
        "mixed": "смешанный лес", "broadleaf": "дубрава", "taiga": "тайга", "river_valley": "речная долина",
        "meadows": "луга", "highlands": "нагорье", "desert": "пустыня", "snowy_mountains": "снежные горы",
        "lake_shore": "берег озера", "sea_coast": "морской берег", "big_river": "большая река",
        "journey": "путешествие",
        // flight.steer_by_head
        "off": "нет", "yaw": "поворотом", "yaw_pitch": "поворотом и наклоном",
    ]

    static func option(_ raw: String) -> String {
        options[raw] ?? raw.replacingOccurrences(of: "_", with: " ")
    }

    private static let units = [
        "m/s": "м/с", "m": "м", "mm/h": "мм/ч", "kW": "кВт", "°C": "°C", "°/s": "°/с", "x": "×",
        "dB": "дБ", "LUFS": "LUFS", "Hz": "Гц",
    ]

    // MARK: Clip groups (AudioEngine.clipGroups)

    /// Group titles and icon keys from the scene (`controls` block), set when a core is made.
    static var clipTitles: [String: String] { ClipGroupLabels.titles }
    static var clipIcons: [String: String] { ClipGroupLabels.icons }

    private static let clipFallbackTitles = [
        "current": "Течение", "whales": "Киты", "dolphins": "Дельфины", "bubbles": "Пузыри",
        "plasma": "Плазма", "flybys": "Пролёты", "beams": "Лучи", "orbits": "Орбиты",
    ]
    private static let clipFallbackIcons = [
        "current": "water", "whales": "whale", "dolphins": "dolphin", "bubbles": "bubbles",
        "plasma": "plasma", "flybys": "flyby", "beams": "beam", "orbits": "orbit",
    ]
    private static let clipLeaves = [
        "level": "Громкость", "events": "События", "motion": "Движение", "distance": "Расстояние",
        "pitch": "Высота тона", "flow": "Скорость течения", "turbulence": "Турбулентность",
        "hiss": "Центр шипения", "whistlers": "Свистящие",
    ]

    /// ("whales", "events") from "clip.whales.events".
    private static func clipParts(_ c: Control) -> (id: String, leaf: String)? {
        guard c.key.hasPrefix("clip.") else { return nil }
        let p = c.key.split(separator: ".").map(String.init)
        return p.count == 3 ? (p[1], p[2]) : nil
    }

    static func clipTitle(_ c: Control) -> String? {
        guard let p = clipParts(c) else { return nil }
        return clipTitles[p.id].flatMap { $0.isEmpty ? nil : $0 } ?? clipFallbackTitles[p.id] ?? sentence(p.id)
    }

    /// A group's lever is named by the group; its aspects by what they change.
    static func clipName(_ c: Control) -> String? {
        guard let p = clipParts(c) else { return nil }
        return c.isMacro ? clipTitle(c) : clipLeaves[p.leaf] ?? sentence(p.leaf)
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

    static func value(_ c: Control) -> String {
        let index = Int(c.value.rounded())
        if c.isDiscrete {
            if index >= 0, index < c.options.count, !c.options[index].isEmpty {
                return option(c.options[index])
            }
            return String(index)
        }
        if c.isLevel {
            if c.isMuted { return "выкл" }
            let db = Int(c.value.rounded())
            return db > 0 ? "+\(db) дБ" : minus("\(db) дБ")
        }
        if let p = clipParts(c) {
            switch p.leaf {
            case "level":
                if c.value <= c.min + 0.01 { return "выкл" }
                let db = Int(c.value.rounded())
                return db > 0 ? "+\(db) дБ" : minus("\(db) дБ")
            case "events" where c.value < 0.01: return "выкл"
            case "motion" where c.value < 0.01: return "стоп"
            case "whistlers": return "+\(Int(c.value.rounded())) /мин"
            default: break
            }
        }
        switch c.key {
        case "foliage.dryness" where c.value < 0: return "по сезону"
        case "scene.motion" where c.value < 0.01: return "стоп"
        case "scene.events" where c.value < 0.01, "scene.bed" where c.value < 0.01, "scene.creatures" where c.value < 0.01:
            return "выкл"
        case "flight.seed": return "№ \(index)"
        case "mix.soloists": return String(index)
        default: break
        }
        if c.unit == "h" {
            let m = Int((c.value * 60).rounded()) % (24 * 60)
            return String(format: "%02d:%02d", m / 60, m % 60)
        }
        if c.unit == "x" {
            return "×" + (c.value >= 10 ? String(format: "%.0f", c.value) : String(format: "%.1f", c.value))
        }
        if c.unit.isEmpty {
            // Unitless: a share of the range, or a factor when 1 is "as built" in the middle.
            if c.min < 0 { return minus((c.value > 0 ? "+" : "") + "\(Int((c.value / Swift.max(c.max, 1) * 100).rounded())) %") }
            if c.max > 1.01 { return "×" + String(format: "%.1f", c.value) }
            return "\(Int((c.fraction * 100).rounded())) %"
        }
        let a = abs(c.value)
        let v = a < 0.005 ? "0" : a >= 10 ? String(format: "%.0f", c.value)
            : a >= 1 ? String(format: "%.1f", c.value) : String(format: "%.2f", c.value)
        return minus("\(v) \(units[c.unit] ?? c.unit)")
    }
}

// MARK: - Sound

/// One cell of the sound section: the group's pictogram (click: mute), its name, a dB rail
/// with a mark at 0 dB, and the value. The mix's clarity takes the last cell and opens its
/// drawer instead.
private struct SoundCell: View {
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
        HStack(spacing: Grid.x1 + 2) {
            Button(action: tapIcon) {
                IconView(icon: Icon.forControl(c, hour: hour),
                         color: muted ? Ink.ash : hovering ? Ink.paper : audible ? Ink.paper.opacity(0.85) : Ink.graphite,
                         size: 16, animating: hovering && meters.live && !muted, struck: muted, lineWidth: 1)
                    .frame(width: 20, height: Popover.soundRowHeight)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .help(isMix ? "Микс: подробно" : muted ? "Включить \(ControlText.group(c).lowercased())" : "Выключить \(ControlText.group(c).lowercased())")
            .accessibilityLabel(isMix ? "Микс: подробно" : muted ? "Включить звук: \(ControlText.group(c))" : "Выключить звук: \(ControlText.group(c))")

            Text(isMix ? ControlText.lever(c) : ControlText.group(c))
                .font(Ink.text(Ink.Size.caption))
                .foregroundStyle(muted ? Ink.ash : hovering || dragging ? Ink.paper : Ink.graphite)
                .lineLimit(1)
                .frame(width: 70, alignment: .leading)

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

            if isMix {
                Button(action: open) {
                    HStack(spacing: 2) {
                        Text(ControlText.value(c))
                        IconView(icon: .chevron, color: hovering ? Ink.graphite : Ink.ash, size: 11, lineWidth: 1)
                    }
                    .frame(width: 52, height: Popover.soundRowHeight, alignment: .trailing)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .focusable(false)
                .foregroundStyle(Ink.paper.opacity(0.85))
            } else {
                Text(ControlText.value(c))
                    .foregroundStyle(muted ? Ink.ash : c.value == 0 ? Ink.graphite : Ink.paper)
                    .frame(width: 52, alignment: .trailing)
            }
        }
        .font(Ink.text(Ink.Size.caption))
        .lineLimit(1)
        .frame(width: Popover.soundColumnWidth)
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .help(isMix ? "Насколько разведены звуки: двойной щелчок — как было" : "Двойной щелчок по шкале — 0 дБ")
        .accessibilityElement(children: .contain)
        .accessibilityLabel(isMix ? ControlText.lever(c) : "Громкость: \(ControlText.group(c))")
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
            ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - 1, width: size.width, height: 2), cornerRadius: 1), with: .color(Ink.hairline))
            ctx.fill(Path(roundedRect: CGRect(x: 0, y: mid - 1, width: x, height: 2), cornerRadius: 1),
                     with: .color(muted ? Ink.hairline : Ink.graphite))
            if control.isLevel {
                ctx.fill(Path(CGRect(x: min(max(zero - 0.5, 0), size.width - 1), y: mid - 4, width: 1, height: 8)),
                         with: .color(Ink.ash))
            }
            let kw: CGFloat = active ? 3 : 2, kh: CGFloat = active ? 14 : 10
            ctx.fill(Path(roundedRect: CGRect(x: min(max(x - kw / 2, 0), size.width - kw), y: mid - kh / 2, width: kw, height: kh),
                          cornerRadius: 1), with: .color(muted ? Ink.ash : Ink.paper))
        }
        .contentShape(Rectangle())
    }
}

// MARK: - World

private struct WorldRow: View {
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
                             size: 18, level: audible ? 0.6 : 0, animating: hovering && meters.live)
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
            .help(hasChildren ? "\(ControlText.group(control)): подробно" : ControlText.group(control))
            .accessibilityLabel(hasChildren ? "\(ControlText.group(control)): подробно" : ControlText.group(control))

            GeometryReader { geo in
                let w = geo.size.width
                let x = CGFloat(control.fraction) * w
                ZStack(alignment: .topLeading) {
                    Color.clear.contentShape(Rectangle())
                    RoundedRectangle(cornerRadius: 1)
                        .fill(Ink.paper)
                        .frame(width: lit ? 3 : 2, height: lit ? 18 : 14)
                        .position(x: min(max(x, 1.5), w - 1.5), y: geo.size.height / 2)
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
                IconView(icon: .chevron, color: lit ? Ink.graphite : Ink.ash, size: 14)
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

/// "Ещё": the hidden groups' pictograms; click to show their levers too.
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
                    Text("Свернуть").foregroundStyle(hovering ? Ink.paper : Ink.graphite)
                } else {
                    Text("Ещё \(more.count):").foregroundStyle(hovering ? Ink.paper : Ink.graphite)
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
        .accessibilityLabel(showAll ? "Показать меньше" : "Показать ещё \(more.count)")
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
                .accessibilityLabel("Назад ко всем рычагам")
                .help("Назад   ⌘←")
                Spacer()
                Text("\(ControlText.lever(macro)): \(ControlText.value(macro))")
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
                .frame(width: 136, alignment: .leading)
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
        .help(control.isOverridden ? "Двойной щелчок по шкале: снова за рычагом «\(ControlText.lever(control))»" : "")
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
            return ControlText.option(control.options[index])
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
