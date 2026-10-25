import CAsmr
import Foundation

/// The scenes the popover offers (core/scenes/*.json, bundled into Resources/scenes), and
/// what the app needs to know about one before and after the core builds it: whether it is a
/// procedural forest (the forest levers, trees and river) or a clip scene (procedural sounds
/// placed and moved in space: their own levers and stage), how it looks, and a name for each
/// clip. Every scene is procedural: the app ships no sound files.
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

    static let forest = SceneEntry(id: "forest", key: N_("Forest"), file: nil)

    /// The three scenes: the built-in forest, and two clip scenes of physical models.
    static let all: [SceneEntry] = [
        forest,
        SceneEntry(id: "underwater", key: N_("Underwater"), file: "underwater_drift.json"),
        SceneEntry(id: "body_scan", key: N_("Body scan"), file: "body_scan.json"),
    ]

    static func with(id: String?) -> SceneEntry? { all.first { $0.id == id } }

    /// The choice remembered across launches.
    static var saved: SceneEntry {
        get {
            // The "· записи" variants are gone: open their scene instead. Any other id that is no
            // longer offered (a removed scene) falls back to the forest.
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
    enum Look { case forest, underwater, void }

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
                let sound = e["sound"] as? [String: Any]
                func f(_ k: String) -> Float? { (motion[k] as? NSNumber)?.floatValue }
                var sweep: (Float, Float, Float, Float)?
                if type == "sweep" { sweep = (f("from_x") ?? -40, f("to_x") ?? 40, f("width") ?? 10, f("height") ?? 0) }
                clips[id] = ClipInfo(id: id, cls: cls, motion: type, nameKey: Scenes.name(files: files, group: e["group"] as? String,
                                                     model: sound?["model"] as? String, preset: sound?["preset"] as? String, cls: cls),
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
            look = .underwater
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

    /// An English name key for a clip (translated when shown): the first keyword found in its
    /// files, its group and its sound model and preset (a procedural clip has no files).
    static func name(files: [String], group: String? = nil, model: String? = nil, preset: String? = nil, cls: ClipInfo.Class) -> String {
        let text = (files + [group, model, preset].compactMap { $0 }).joined(separator: " ").lowercased()
        let words: [(String, String)] = [
            // Under water.
            ("humpback_dolphin", N_("dolphins")), ("humpback", N_("humpback")), ("blue_whale", N_("blue whale")), ("fin_whale", N_("fin whale")),
            ("sperm_whale", N_("sperm whale")), ("whale", N_("whale")), ("dolphin", N_("dolphins")), ("bubbles", N_("bubbles")),
            ("swimmer", N_("swimmer")), ("reef", N_("reef")), ("shrimp", N_("shrimp")), ("ocean", N_("ocean")),
            ("rain_on_surface", N_("rain on the surface")), ("fish_grunt", N_("fish")), ("fish", N_("fish")),
            ("current", N_("current")), ("rain", N_("rain")),
            // The body scan.
            ("guide", N_("guide")), ("bowl", N_("singing bowl")), ("tingsha", N_("tingsha")),
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
