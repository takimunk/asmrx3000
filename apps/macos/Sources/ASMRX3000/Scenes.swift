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
    /// Russian, as the picker shows it.
    let title: String
    /// File in the scenes folder; nil = the core's built-in forest by the river.
    let file: String?

    static let all: [SceneEntry] = [
        SceneEntry(id: "forest", title: "Лес", file: nil),
        SceneEntry(id: "underwater", title: "Под водой", file: "underwater_drift.json"),
        SceneEntry(id: "space", title: "Космос", file: "space_still.json"),
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
    /// Russian, for the selection chip: "кит", "дельфины", "комета"…
    var name: String
    var emitters: Int
    var spread: Float
    /// A sweep's lateral width far from the screen and its height, a pass into the screen.
    var sweep: (fromX: Float, toX: Float, width: Float, height: Float)?
    var toScreen: Bool

    var moves: Bool { motion != "fixed" }

    /// What the selection chip says about how it moves.
    var motionText: String {
        switch motion {
        case "fixed": return "Звучит на месте"
        case "escort": return "Держится рядом с вами"
        case "drift": return "Проплывает мимо, пока вы плывёте вперёд"
        case "pass": return toScreen ? "Проходит мимо и уходит в экран" : "Проходит мимо"
        case "orbit": return "Кружит вокруг"
        case "sweep": return "Волна сквозь голову и экран"
        case "portal": return "Втягивается в экран"
        default: return "Движется сам"
        }
    }
}

/// A scene as loaded: its JSON (nil for the built-in forest) and what the shell reads from it.
struct SceneInfo {
    enum Look { case forest, underwater, space, void }

    var entry: SceneEntry
    /// Title shown by the picker (the entry's, or the JSON's name for a `--scene` file).
    var title: String
    var json: String?
    var look: Look
    var clips: [UInt32: ClipInfo]
    /// ASMR_GROUP_* of the procedural sources the scene has besides its clips.
    var groups: Set<Int>
    /// Bottom of the water or the ground, m (environment.ground_z).
    var groundZ: Float

    /// Clip scenes get their own levers and stage; the forest levers do nothing there.
    var isClip: Bool { !clips.isEmpty }

    static let forest = SceneInfo(entry: .forest, title: SceneEntry.forest.title, json: nil, look: .forest, clips: [:],
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
            info.title = n
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
                clips[id] = ClipInfo(id: id, cls: cls, motion: type, name: Scenes.name(files: files, cls: cls),
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
        } else if env["medium"] as? String == "water" || entry.id.hasPrefix("underwater") {
            look = .underwater
        } else if entry.id.hasPrefix("space") || (root["name"] as? String ?? "").lowercased().contains("space") {
            look = .space
        } else {
            look = .void
        }
        return SceneInfo(entry: entry, title: entry.title, json: json, look: look, clips: clips, groups: groups,
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

    /// A Russian name from the clip's files: the first keyword that matches.
    static func name(files: [String], cls: ClipInfo.Class) -> String {
        let text = files.joined(separator: " ").lowercased()
        let words: [(String, String)] = [
            ("humpback_dolphin", "дельфины"), ("humpback", "горбач"), ("blue_whale", "синий кит"), ("fin_whale", "финвал"),
            ("sperm_whale", "кашалот"), ("whale", "кит"), ("dolphin", "дельфины"), ("bubbles", "пузыри"),
            ("swimmer", "пловец"), ("reef", "риф"), ("ocean", "океан"), ("comet", "комета"), ("juno", "пролёт «Юноны»"),
            ("pulsar", "пульсар"), ("chorus", "хор магнитосферы"), ("whistler", "свистящие атмосферики"),
            ("black_hole", "чёрная дыра"), ("m87", "чёрная дыра"), ("sgra", "чёрная дыра"), ("shimmer", "мерцание"),
            ("swell", "космическая волна"), ("saturn", "Сатурн"), ("voyager", "межзвёздная плазма"),
            ("plasma", "плазма"), ("drone", "гул"),
        ]
        if let hit = words.first(where: { text.contains($0.0) }) { return hit.1 }
        switch cls {
        case .bed: return "фон"
        case .texture: return "текстура"
        case .creature: return "существо"
        case .event: return "событие"
        }
    }
}

/// Titles and icon keys of the current scene's clip groups, as the scene names them: written
/// by the engine when it loads a scene, read by the levers (kept here, outside the UI, so the
/// audio layer builds on its own).
enum ClipGroupLabels {
    static var titles: [String: String] = [:]
    static var icons: [String: String] = [:]
}
