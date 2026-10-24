import CAsmr
import Foundation

/// The scenes the popover offers (core/scenes/*.json, bundled into Resources/scenes), and
/// what the app needs to know about one before and after the core builds it: whether it is a
/// procedural forest (the forest levers, trees and river) or a clip scene (recorded or
/// generated sounds placed and moved in space: their own levers and stage), how it looks, and
/// a name for each clip.
struct SceneEntry: Identifiable, Equatable {
    /// Stored in UserDefaults ("scene").
    let id: String
    /// The picker's title: an English key (translated when shown), or, for a `--scene` file,
    /// the file's own name, shown verbatim.
    let titleKey: String
    private let verbatim: Bool
    var title: String { verbatim ? titleKey : tr(titleKey) }
    /// File in the scenes folder; nil = the core's built-in forest by the river.
    let file: String?

    /// A scene whose title is shown as given (a `--scene` file's name).
    init(id: String, title: String, file: String?) {
        self.id = id; titleKey = title; verbatim = true; self.file = file
    }

    /// A catalogue scene: `key` is the English title, translated at display time.
    init(id: String, key: String, file: String?) {
        self.id = id; titleKey = key; verbatim = false; self.file = file
    }

    static let all: [SceneEntry] = [
        SceneEntry(id: "forest", key: N_("Forest"), file: nil),
        SceneEntry(id: "underwater", key: N_("Underwater"), file: "underwater_drift.json"),
        SceneEntry(id: "space", key: N_("Space"), file: "space_still.json"),
        SceneEntry(id: "lighthouse", key: N_("Lighthouse"), file: "lighthouse_gale.json"),
        SceneEntry(id: "night_train", key: N_("Night train"), file: "night_train.json"),
        SceneEntry(id: "singing_ice", key: N_("Singing ice"), file: "singing_ice.json"),
        SceneEntry(id: "tea_house", key: N_("Tea house"), file: "tea_house.json"),
        SceneEntry(id: "night_library", key: N_("Night library"), file: "night_library.json"),
        SceneEntry(id: "body_scan", key: N_("Body scan"), file: "body_scan.json"),
        SceneEntry(id: "cave_drips", key: N_("Cave"), file: "cave_drips.json"),
        SceneEntry(id: "sailboat_anchor", key: N_("Yacht at anchor"), file: "sailboat_anchor.json"),
        SceneEntry(id: "campfire_shore", key: N_("Campfire by the sea"), file: "campfire_shore.json"),
        SceneEntry(id: "mountain_tent", key: N_("Tent in the rain"), file: "mountain_tent.json"),
    ]

    static let forest = all[0]

    static func with(id: String?) -> SceneEntry? { all.first { $0.id == id } }

    /// The choice remembered across launches.
    static var saved: SceneEntry {
        get {
            // The "· записи" variants are no longer offered: open their scene instead.
            let id = UserDefaults.standard.string(forKey: "scene").map { $0.hasSuffix("_real") ? String($0.dropLast(5)) : $0 }
            return with(id: id) ?? forest
        }
        set { UserDefaults.standard.set(newValue.id, forKey: "scene") }
    }
}

/// One clip entity of the scene, as its JSON describes it (ids match asmr_get_objects).
struct ClipInfo {
    enum Class: String { case bed, texture, creature, event }
    var id: UInt32
    var cls: Class
    /// fixed, pass, drift, escort, orbit, sweep, portal.
    var motion: String
    /// English key of the name for the selection chip ("whale", "dolphins", "comet"…).
    var nameKey: String
    var name: String { tr(nameKey) }
    var emitters: Int
    var spread: Float
    /// A sweep's lateral width far from the screen and its height, a pass into the screen.
    var sweep: (fromX: Float, toX: Float, width: Float, height: Float)?
    var toScreen: Bool

    var moves: Bool { motion != "fixed" }

    /// What the selection chip says about how it moves.
    var motionText: String {
        switch motion {
        case "fixed": return tr("Sounds in place")
        case "escort": return tr("Stays close to you")
        case "drift": return tr("Drifts past as you swim forward")
        case "pass": return toScreen ? tr("Passes by and goes into the screen") : tr("Passes by")
        case "orbit": return tr("Circles around you")
        case "sweep": return tr("A wave through your head and the screen")
        case "portal": return tr("Pulled into the screen")
        default: return tr("Moves on its own")
        }
    }
}

/// A scene as loaded: its JSON (nil for the built-in forest) and what the shell reads from it.
struct SceneInfo {
    enum Look { case forest, underwater, space, void }

    var entry: SceneEntry
    /// The JSON's name for a `--scene` file; otherwise the entry's title is shown.
    var customTitle: String?
    var title: String { customTitle ?? entry.title }
    var json: String?
    var look: Look
    var clips: [UInt32: ClipInfo]
    /// ASMR_GROUP_* of the procedural sources the scene has besides its clips.
    var groups: Set<Int>
    /// Bottom of the water or the ground, m (environment.ground_z).
    var groundZ: Float

    /// Clip scenes get their own levers and stage; the forest levers do nothing there.
    var isClip: Bool { !clips.isEmpty }

    static let forest = SceneInfo(entry: .forest, json: nil, look: .forest, clips: [:],
                                  groups: [], groundZ: -1.7)

    /// Reads a scene file from the scenes folder; nil when it is missing or unreadable.
    static func load(_ entry: SceneEntry) -> SceneInfo? {
        guard let file = entry.file else { return .forest }
        guard let dir = Scenes.directory,
              let json = try? String(contentsOf: dir.appendingPathComponent(file), encoding: .utf8) else { return nil }
        return parse(json, entry: entry)
    }

    /// A `--scene path` file: matched to the catalogue by file name, otherwise named by its JSON.
    static func load(path: String) -> SceneInfo? {
        guard let json = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        let name = (path as NSString).lastPathComponent
        let entry = SceneEntry.all.first { $0.file == name } ?? SceneEntry(id: "file:" + path, title: name, file: nil)
        var info = parse(json, entry: entry)
        if entry.file == nil, let n = (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])?["name"] as? String {
            info.customTitle = n
        }
        return info
    }

    static func parse(_ json: String, entry: SceneEntry) -> SceneInfo {
        let root = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
        let env = root["environment"] as? [String: Any] ?? [:]
        let entities = root["entities"] as? [[String: Any]] ?? []
        var clips: [UInt32: ClipInfo] = [:]
        var groups = Set<Int>()
        for (i, e) in entities.enumerated() {
            let kind = e["kind"] as? String ?? ""
            // Same ids as the core: explicit, or 1000 + index (scene.rs, World::build).
            let id = (e["id"] as? NSNumber)?.uint32Value ?? UInt32(1000 + i)
            switch kind {
            case "clip":
                let motion = e["motion"] as? [String: Any] ?? [:]
                let type = motion["type"] as? String ?? "fixed"
                let cls = ClipInfo.Class(rawValue: e["class"] as? String ?? "bed") ?? .bed
                let files = e["files"] as? [String] ?? []
                func f(_ k: String) -> Float? { (motion[k] as? NSNumber)?.floatValue }
                var sweep: (Float, Float, Float, Float)?
                if type == "sweep" { sweep = (f("from_x") ?? -40, f("to_x") ?? 40, f("width") ?? 10, f("height") ?? 0) }
                clips[id] = ClipInfo(id: id, cls: cls, motion: type, nameKey: Scenes.name(files: files, cls: cls),
                                     emitters: (e["emitters"] as? NSNumber)?.intValue ?? 1,
                                     spread: (e["spread"] as? NSNumber)?.floatValue ?? 0,
                                     sweep: sweep, toScreen: motion["to_screen"] as? Bool ?? false)
            case "tree": groups.formUnion([ASMR_GROUP_WIND, ASMR_GROUP_FOLIAGE])
            case "river": groups.insert(ASMR_GROUP_WATER)
            case "bird": groups.insert(ASMR_GROUP_BIRDS)
            case "source":
                let model = (e["sound"] as? [String: Any])?["model"] as? String ?? e["model"] as? String ?? ""
                switch model {
                case "water": groups.insert(ASMR_GROUP_WATER)
                case "wind": groups.insert(ASMR_GROUP_WIND)
                case "rain", "thunder": groups.insert(ASMR_GROUP_RAIN)
                case "insects": groups.insert(ASMR_GROUP_INSECTS)
                case "fire": groups.insert(ASMR_GROUP_FIRE)
                default: break
                }
            default: break
            }
        }
        let look: Look
        if clips.isEmpty {
            look = .forest
        } else if entry.id.hasPrefix("underwater") || (root["name"] as? String ?? "").lowercased().contains("underwater") {
            // Not the medium alone: the lighthouse is muffled like water but stands on a rock.
            look = .underwater
        } else if entry.id.hasPrefix("space") || (root["name"] as? String ?? "").lowercased().contains("space") {
            look = .space
        } else {
            look = .void
        }
        return SceneInfo(entry: entry, json: json, look: look, clips: clips, groups: groups,
                         groundZ: (env["ground_z"] as? NSNumber)?.floatValue ?? -1.7)
    }
}

enum Scenes {
    /// Resources/scenes inside the app; core/scenes in the source tree for runs from .build.
    static let directory: URL? = {
        if let r = Bundle.main.resourceURL?.appendingPathComponent("scenes"), FileManager.default.fileExists(atPath: r.path) {
            return r
        }
        let dev = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../../core/scenes").standardizedFileURL
        return FileManager.default.fileExists(atPath: dev.path) ? dev : nil
    }()

    /// Points the core at the bundled sounds (Resources/sounds) before the first engine is made,
    /// unless ASMR_SOUNDS_DIR is already set. Runs from .build keep the core's compile-time
    /// fallback (the repository's assets/sounds).
    static func prepareEnvironment() {
        guard let r = Bundle.main.resourceURL?.appendingPathComponent("sounds"),
              FileManager.default.fileExists(atPath: r.path) else { return }
        setenv("ASMR_SOUNDS_DIR", r.path, 0)
    }

    /// An English name key from the clip's files (translated when shown): the first keyword that matches.
    static func name(files: [String], cls: ClipInfo.Class) -> String {
        let text = files.joined(separator: " ").lowercased()
        let words: [(String, String)] = [
            ("humpback_dolphin", N_("dolphins")), ("humpback", N_("humpback")), ("blue_whale", N_("blue whale")), ("fin_whale", N_("fin whale")),
            ("sperm_whale", N_("sperm whale")), ("whale", N_("whale")), ("dolphin", N_("dolphins")), ("bubbles", N_("bubbles")),
            ("swimmer", N_("swimmer")), ("reef", N_("reef")), ("ocean", N_("ocean")), ("comet", N_("comet")), ("juno", N_("Juno flyby")),
            ("pulsar", N_("pulsar")), ("chorus", N_("magnetosphere chorus")), ("whistler", N_("whistlers")),
            ("black_hole", N_("black hole")), ("m87", N_("black hole")), ("sgra", N_("black hole")), ("shimmer", N_("shimmer")),
            ("swell", N_("cosmic swell")), ("saturn", N_("Saturn")), ("voyager", N_("interstellar plasma")),
            ("plasma", N_("plasma")), ("drone", N_("drone")),
            // The station, its ships and docking.
            ("hull_hum", N_("station hull")), ("life_support", N_("life support")), ("distant_pump", N_("pump")),
            ("hull_creak", N_("hull creak")), ("hull_tick", N_("hull ticks")), ("freighter", N_("freighter")),
            ("medium_ship", N_("ship")), ("shuttle", N_("shuttle")), ("convoy", N_("convoy")), ("rcs_", N_("thrusters")),
            ("clamp", N_("docking clamp")), ("airlock", N_("airlock")), ("robot_arm", N_("robot arm")),
            ("radio_hiss", N_("radio")), ("console_beep", N_("console")),
            // The lighthouse.
            ("optic", N_("lens")), ("clockwork", N_("clockwork")), ("rain_on_glass", N_("rain on the glass")),
            ("surf_crash", N_("surf")), ("bell_buoy", N_("bell buoy")), ("foghorn", N_("foghorn")),
            ("pane_rattle", N_("rattling pane")), ("stair", N_("stairs")),
            // The night train.
            ("car_interior", N_("carriage")), ("rolling_noise", N_("wheels on rails")), ("bogie", N_("bogie")),
            ("crossing", N_("level crossing")), ("passing_train", N_("passing train")), ("bridge", N_("bridge")),
            ("tunnel", N_("tunnel")), ("rain_window", N_("rain on the window")), ("tea_glass", N_("tea glass")),
            ("panel_creak", N_("creaking panel")), ("vent_hum", N_("vent")), ("loco_horn", N_("horn")),
            ("station_pass", N_("station")),
            // The body scan.
            ("guide_", N_("guide")), ("bowl_strike", N_("singing bowl")), ("tingsha", N_("tingsha")),
            // The tea house.
            ("kettle", N_("kettle")), ("fukusa", N_("silk cloth")), ("ladle", N_("ladle")), ("whisk", N_("whisk")),
            ("tatami", N_("host")), ("bowl_slide", N_("tea bowl")), ("suikinkutsu", N_("water basin")),
            ("shishi", N_("bamboo knocker")), ("furin", N_("wind chime")), ("temple_bell", N_("temple bell")),
            ("rain_on_surface", N_("rain on the surface")), ("fish_grunt", N_("fish")),
            // The yacht at anchor.
            ("cabin_lap", N_("water at the hull")), ("hull_slap", N_("slaps on the hull")), ("hull_cluck", N_("clucks under the hull")),
            ("cabin_creak", N_("creaking cabin")), ("rope_creak", N_("snubber line")), ("halyard_tap", N_("halyard on the mast")),
            ("halyard_clink", N_("halyard")), ("halyard_rattle", N_("halyard in a gust")), ("chain_grumble", N_("anchor chain")),
            ("bay_halyards", N_("moored boats")), ("bay_rigging", N_("rigging in the wind")), ("yacht_gull", N_("gull")),
            // The campfire by the sea.
            ("campfire_crackle", N_("campfire")), ("campfire_flare", N_("flames")), ("campfire_bonfire", N_("flames")),("campfire_log", N_("log")),
            ("campfire_damp", N_("damp driftwood")), ("shore_breakers", N_("breakers")), ("shore_wave", N_("wave")),
            ("shore_curlew", N_("curlew")),
            // The night library.
            ("hearth_crackle", N_("fire")), ("hearth_embers", N_("embers")), ("log_settle", N_("settling logs")),
            ("tallwindow_squall", N_("rain lashing the windows")), ("tallwindow", N_("rain on the windows")),
            ("page_turn", N_("pages")), ("pen_scratch", N_("pen")), ("clock_tick", N_("clock")), ("clock_strike", N_("clock strikes")),
            ("floor_creak", N_("floorboards")), ("shelf_creak", N_("bookcase")), ("radiator", N_("radiator")), ("door_far", N_("distant door")),
            // The tent in the rain.
            ("tent_fly_", N_("rain on the fly")), ("tent_drips", N_("drips off the fly")), ("tent_flap", N_("flapping fly")), ("tent_pole_creak", N_("tent pole")),
            ("tent_thunder_roll", N_("far thunder")), ("tent_creek", N_("mountain stream")),
            ("tent_bag_rustle", N_("sleeping bag")), ("tent_zipper", N_("zipper")),
            // The singing ice, last: "bowl_strike" and "bowl_slide" hold "owl_".
            ("ice_sing", N_("singing ice")), ("ice_crack", N_("crack")), ("ice_run", N_("running crack")),
            ("ice_boom", N_("ice boom")), ("owl_", N_("owl")), ("skater", N_("skater")),
            // The cave.
            ("cave_plink", N_("drip into a pool")), ("cave_slowdrip", N_("slow drip")), ("cave_deepdrip", N_("far drip")),
            ("cave_rockdrip", N_("drip on rock")), ("cave_pebbles", N_("pebbles")), ("cave_stream", N_("underground stream")),
            ("cave_runnel", N_("runnel")),
        ]
        if let hit = words.first(where: { text.contains($0.0) }) { return hit.1 }
        switch cls {
        case .bed: return N_("background")
        case .texture: return N_("texture")
        case .creature: return N_("creature")
        case .event: return N_("event")
        }
    }
}

/// Titles and icon keys of the current scene's clip groups, as the scene names them: written
/// by the engine when it loads a scene, read by the levers (kept here, outside the UI, so the
/// audio layer builds on its own).
enum ClipGroupLabels {
    static var titles: [String: String] = [:]
    static var icons: [String: String] = [:]
    /// The scene levers' titles and icon keys, by lever key.
    static var leverTitles: [String: String] = [:]
    static var leverIcons: [String: String] = [:]
}
