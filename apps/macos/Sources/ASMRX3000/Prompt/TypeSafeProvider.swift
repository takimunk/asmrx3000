import Foundation

/// TypeSafe System One (Jev): POST https://api.typesafe.ai/v1/systemone. Jev answers typed
/// questions (choice / score / noul) with probabilities instead of writing JSON, so this
/// adapter asks one Choice per control in a single request ("fan-out"): keep it, move it a
/// little / plainly / a lot either way, or jump to a physical anchor ("gale", "03:00",
/// "small_waterfall"). Code turns the answers into the core's patch and writes the
/// explanation line itself. Key: keychain, account "typesafe".
struct TypeSafeProvider: PromptProvider {
    let name = "typesafe"
    let key: String
    var model = "jev-latest"
    static let endpoint = URL(string: "https://api.typesafe.ai/v1/systemone")!
    /// An answer whose winning option is less likely than this is ignored.
    static let minProbability = 0.4

    func complete(_ request: PromptRequest) async throws -> PromptReply {
        let scene = Scene(state: request.state)
        let knobs = Knob.all.filter { scene.values[$0.key] != nil && ($0.key != "fire.heat" || scene.kinds.contains("fire")) }
        var questions: [String: Any] = [:]
        for k in knobs {
            questions[k.key] = ["type": "choice", "instructions": k.instructions, "criteria": k.criteria]
        }
        questions["glide"] = ["type": "choice", "instructions": Self.glideInstructions, "criteria": Self.glideOptions]
        questions["birds.side"] = ["type": "choice", "instructions": Self.sideInstructions, "criteria": Self.sideOptions]
        let body: [String: Any] = [
            "model": model,
            "state": ["request": request.text, "scene": scene.summary],
            "questions": questions,
        ]
        let answers = try await post(body)

        var changes: [[String: Any]] = []
        var said: [String] = []
        let russian = request.text.range(of: "\\p{Cyrillic}", options: .regularExpression) != nil
        for k in knobs {
            guard let current = scene.values[k.key] else { continue }
            let target: Knob.Target?
            if k.key == "env.time_of_day" {
                target = Self.hour(answers[k.key]).map { Knob.Target(value: $0, label: String(format: "%02d:%02d", Int($0), Int(($0 - $0.rounded(.down)) * 60)), relative: nil) }
            } else if let pick = Self.change(answers[k.key], maxUnchanged: k.preset ? 0.25 : 0.5) {
                target = k.target(pick, current)
            } else {
                target = nil
            }
            guard let target else { continue }
            let value: Any = k.preset ? target.label : target.value
            if !k.preset, abs(target.value - current) < 1e-3 { continue }
            changes.append(["key": k.key, "value": value])
            said.append(k.describe(target, current, russian))
        }
        var moves: [[String: Any]] = []
        if let side = Self.pick(answers["birds.side"]), side != "none", let tree = scene.nearestMovableTree(to: side) {
            moves.append(["object": tree.id, "position": tree.position])
            said.append(russian ? "птицы \(Self.sideRu[side] ?? side)" : "birds \(side)")
        }
        let glide = Self.glideSeconds[Self.pick(answers["glide"]) ?? "normal"] ?? 4
        let explanation = said.isEmpty
            ? (russian ? "Не нашёл, что тут изменить." : "Nothing to change for that.")
            : said.joined(separator: ", ").capitalizedFirst + "."
        let patch: [String: Any] = ["explanation": explanation, "glide_s": glide, "changes": changes, "moves": moves]
        return try PromptReply(patch: JSONSerialization.data(withJSONObject: patch))
    }

    /// The option to move to: only when "unchanged" is unlikely, then the likeliest other one.
    static func change(_ answer: Any?, maxUnchanged: Double) -> String? {
        guard let a = answer as? [String: Any], let p = a["probabilities"] as? [String: Double] else { return nil }
        guard (p["unchanged"] ?? 0) < maxUnchanged else { return nil }
        return p.filter { $0.key != "unchanged" }.max { $0.value < $1.value }?.key
    }

    /// Time of day: the probability spreads over neighbouring half hours, so take their
    /// circular mean (if "unchanged" is unlikely).
    static func hour(_ answer: Any?) -> Double? {
        guard let a = answer as? [String: Any], let p = a["probabilities"] as? [String: Double], (p["unchanged"] ?? 0) < 0.5 else { return nil }
        var x = 0.0, y = 0.0
        for (label, w) in p where label != "unchanged" {
            let parts = label.split(separator: ":").compactMap { Double($0) }
            guard parts.count == 2 else { continue }
            let angle = (parts[0] + parts[1] / 60) / 24 * 2 * .pi
            x += w * cos(angle)
            y += w * sin(angle)
        }
        guard x * x + y * y > 0.04 else { return nil }
        let h = atan2(y, x) / (2 * .pi) * 24
        return ((h < 0 ? h + 24 : h) * 2).rounded() / 2
    }

    /// Winning option of a choice answer if the model is sure enough.
    static func pick(_ answer: Any?) -> String? {
        guard let a = answer as? [String: Any], let choice = a["choice"] as? String else { return nil }
        let p = (a["probabilities"] as? [String: Any])?[choice] as? Double ?? 1
        return p >= minProbability ? choice : nil
    }

    private func post(_ body: [String: Any]) async throws -> [String: Any] {
        var req = URLRequest(url: Self.endpoint, timeoutInterval: 8)
        req.httpMethod = "POST"
        req.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        // One quick retry on overload / rate limit; the latency budget allows no more.
        for attempt in 0..<2 {
            let (data, response) = try await URLSession.shared.data(for: req)
            let code = (response as? HTTPURLResponse)?.statusCode ?? 0
            if (code == 429 || code == 529) && attempt == 0 {
                try await Task.sleep(nanoseconds: 300_000_000)
                continue
            }
            guard code == 200 else {
                // The body names the failing field on 4xx; never contains the key.
                throw PromptError.http(code, String(decoding: data.prefix(300), as: UTF8.self))
            }
            guard let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let answers = o["answers"] as? [String: Any]
            else { throw PromptError.badReply("no answers") }
            return answers
        }
        throw PromptError.http(429, "")
    }

    // MARK: Pace and side

    static let glideInstructions = "The listener asked to change a forest-by-a-river soundscape (`request`). How fast should the change happen?"
    static let glideOptions: [String: Any] = [
        "normal": "No pace is mentioned.",
        "gradual": "Slowly, gradually, fading, dying down, building up over time (e.g. 'rain is easing off', 'стихает', 'постепенно').",
        "instant": "Right now, suddenly, at once.",
    ]
    static let glideSeconds: [String: Double] = ["normal": 4, "gradual": 25, "instant": 0.8]

    static let sideInstructions = "The listener asked to change a forest-by-a-river soundscape (`request`). Does the request say on which side of the listener the birds should be?"
    static let sideOptions: [String: Any] = [
        "none": "No side is mentioned for birds.",
        "left": "Birds on the left ('слева').",
        "right": "Birds on the right ('справа').",
        "ahead": "Birds in front ('впереди').",
        "behind": "Birds behind ('сзади').",
    ]
    static let sideRu = ["left": "слева", "right": "справа", "ahead": "впереди", "behind": "сзади"]
}

/// Current values and movable trees from asmr_get_state_json.
private struct Scene {
    var values: [String: Double] = [:]
    var units: [String: String] = [:]
    var options: [String: [String]] = [:]
    var trees: [(id: Int, pos: [Double])] = []
    /// Object kinds present ("tree", "river", "fire", …).
    var kinds: Set<String> = []

    init(state: Data) {
        guard let o = try? JSONSerialization.jsonObject(with: state) as? [String: Any] else { return }
        for c in o["controls"] as? [[String: Any]] ?? [] {
            guard let k = c["key"] as? String, let v = c["value"] as? Double else { continue }
            values[k] = v
            units[k] = c["unit"] as? String
            options[k] = c["options"] as? [String]
        }
        for obj in o["objects"] as? [[String: Any]] ?? [] {
            if let k = obj["kind"] as? String { kinds.insert(k) }
        }
        for obj in o["objects"] as? [[String: Any]] ?? [] where obj["kind"] as? String == "tree" && obj["movable"] as? Bool == true {
            if let id = obj["id"] as? Int, let p = obj["pos"] as? [Double], p.count == 3 { trees.append((id, p)) }
        }
    }

    /// Plain "key: value unit" map the model reads as state.
    var summary: [String: String] {
        var s: [String: String] = [:]
        for (k, v) in values {
            if let opts = options[k], Int(v.rounded()) < opts.count {
                s[k] = opts[Int(v.rounded())]
            } else {
                s[k] = String(format: "%g", (v * 100).rounded() / 100) + ((units[k] ?? "").isEmpty ? "" : " " + units[k]!)
            }
        }
        return s
    }

    /// The movable tree closest to `side`, and where to put it (6 m out that way).
    func nearestMovableTree(to side: String) -> (id: Int, position: [Double])? {
        let dir: (Double, Double) = ["left": (0, 1), "right": (0, -1), "ahead": (1, 0), "behind": (-1, 0)][side] ?? (0, 1)
        guard let t = trees.max(by: { $0.pos[0] * dir.0 + $0.pos[1] * dir.1 < $1.pos[0] * dir.0 + $1.pos[1] * dir.1 }) else { return nil }
        let x = dir.0 == 0 ? t.pos[0] : 6 * dir.0
        let y = dir.1 == 0 ? t.pos[1] : 6 * dir.1
        return (t.id, [x, y, t.pos[2]])
    }
}

/// One control as a Choice: relative steps, physical anchors, or presets.
private struct Knob {
    enum Step { case add(Double), times(Double) }

    let key: String
    let what: String
    let ru: String
    var step: Step?
    var floor: Double = 0
    var range: ClosedRange<Double> = 0...1
    /// name → (value, description)
    var anchors: [(String, Double, String)] = []
    /// Preset controls: option names → descriptions.
    var presets: [(String, String)] = []

    var preset: Bool { !presets.isEmpty }

    /// Short English label: the group ("wind", "birds"), or the parameter for env/storm/regime.
    var en: String {
        let parts = key.split(separator: ".").map(String.init)
        if key == "rain.storm" { return "thunder" }
        if parts[0] == "env" || key == "water.regime" { return parts[1].replacingOccurrences(of: "_", with: " ") }
        return parts[0]
    }

    static let relative: [(String, Int, String)] = [
        ("much_less", -3, "much less / much lower / far quieter"),
        ("less", -2, "less / lower / calmer than now"),
        ("a_bit_less", -1, "slightly less"),
        ("a_bit_more", 1, "slightly more"),
        ("more", 2, "more / stronger / louder than now"),
        ("much_more", 3, "much more / far stronger"),
    ]

    var instructions: String {
        "The listener asked to change a forest-by-a-river soundscape; their words are in `request` (often Russian), current values in `scene`. " +
            "How should \(what) (`scene.\(key)`) change to satisfy the request? Overall moods count: 'calmer' lowers wind, rain, river flow, birds and volume a little; " +
            "'night' means few birds and more insects; a storm brings rain and wind. Pick `unchanged` if the request implies nothing about it."
    }

    var criteria: [String: Any] {
        var c: [String: Any] = ["unchanged": "Leave it as it is."]
        if preset {
            for (n, d) in presets { c[n] = d }
            return c
        }
        if step != nil {
            for (n, _, d) in Self.relative { c[n] = d }
        }
        for (n, v, d) in anchors { c[n] = "\(d) (\(String(format: "%g", v)))" }
        return c
    }

    struct Target { var value: Double; var label: String; var relative: Int? }

    func target(_ pick: String, _ current: Double) -> Target? {
        if preset { return presets.contains { $0.0 == pick } ? Target(value: 0, label: pick, relative: nil) : nil }
        if let a = anchors.first(where: { $0.0 == pick }) { return Target(value: a.1, label: pick, relative: nil) }
        guard let k = Self.relative.first(where: { $0.0 == pick })?.1, let step else { return nil }
        let v: Double
        switch step {
        case let .add(d): v = current + d * Double(k)
        case let .times(f): v = k > 0 ? max(current, floor) * pow(f, Double(k)) : current * pow(f, Double(k))
        }
        return Target(value: min(max(v, range.lowerBound), range.upperBound), label: pick, relative: k)
    }

    func describe(_ t: Target, _ current: Double, _ russian: Bool) -> String {
        let name = russian ? ru : en
        if preset { return "\(name): \(t.label.replacingOccurrences(of: "_", with: " "))" }
        if key == "env.time_of_day" { return t.label }
        return "\(name) " + (t.value > current ? "↑" : "↓")
    }

    static let all: [Knob] = [
        Knob(key: "water.flow", what: "the river's flow (discharge: how busy and loud the water is)", ru: "река",
             step: .add(0.1), anchors: [("dry", 0.05, "almost no flow"), ("low_brook", 0.35, "low summer brook"),
                                         ("as_built", 0.6, "normal"), ("high_water", 0.8, "high water"), ("flood", 1, "spring flood")]),
        Knob(key: "water.regime", what: "the character of the water", ru: "вода",
             presets: [("scene", "As the scene was built."), ("still_lake", "A still lake."), ("lake_lapping", "Small waves lapping a lake shore."),
                       ("slow_river", "A deep slow river."), ("brook_pebbles", "A babbling pebble brook."), ("brook_rocks", "A splashy rocky brook."),
                       ("rapids", "Roaring rapids, white water."), ("small_waterfall", "A small waterfall nearby."),
                       ("drips", "Cave-like water drips."), ("trickle", "A thin trickle.")]),
        Knob(key: "wind.speed", what: "the wind", ru: "ветер", step: .times(1.35), floor: 1.5, range: 0...30,
             anchors: [("calm", 0.3, "dead calm"), ("light_air", 1.5, "light air"), ("gentle_breeze", 4, "gentle breeze"),
                       ("fresh_wind", 8, "fresh wind"), ("strong_wind", 12, "strong wind"), ("gale", 17, "gale"), ("storm_wind", 25, "storm wind")]),
        Knob(key: "rain.rate", what: "the rain", ru: "дождь", step: .times(1.8), floor: 0.6, range: 0...100,
             anchors: [("dry", 0, "no rain"), ("drizzle", 0.3, "drizzle"), ("light_rain", 1, "light rain"), ("steady_rain", 4, "steady rain"),
                       ("heavy_rain", 10, "heavy rain"), ("downpour", 25, "downpour"), ("cloudburst", 60, "cloudburst")]),
        Knob(key: "rain.storm", what: "thunder and lightning", ru: "гроза", step: .add(0.2),
             anchors: [("no_thunder", 0, "no thunder"), ("distant_rumbles", 0.2, "distant rumbles"),
                       ("storm_nearby", 0.6, "thunderstorm nearby"), ("overhead", 0.9, "storm right overhead")]),
        Knob(key: "birds.activity", what: "how much the birds sing", ru: "птицы", step: .add(0.15),
             anchors: [("silent", 0, "no birdsong"), ("few_birds", 0.25, "a few birds"), ("normal", 0.6, "normal"), ("full_chorus", 1, "full chorus")]),
        Knob(key: "insects.activity", what: "crickets and other insects", ru: "насекомые", step: .times(1.4), floor: 0.5, range: 0...2,
             anchors: [("silent", 0, "no insects"), ("normal", 1, "as in the scene"), ("loud", 2, "loud crickets")]),
        Knob(key: "master.volume", what: "the overall volume", ru: "громкость", step: .add(0.07), range: 0...1.5),
        Knob(key: "env.temperature", what: "the air temperature", ru: "температура", step: .add(3), range: -20...40,
             anchors: [("freezing", -5, "freezing"), ("cold", 5, "cold"), ("cool", 12, "cool"), ("mild", 18, "mild"), ("warm", 24, "warm"), ("hot", 32, "hot")]),
        Knob(key: "env.season", what: "the season", ru: "сезон",
             presets: [("spring", "Spring."), ("summer", "Summer."), ("autumn", "Autumn, fall."), ("winter", "Winter.")]),
        Knob(key: "fire.heat", what: "the campfire", ru: "костёр", step: .times(1.8), floor: 5, range: 1...1000,
             anchors: [("embers", 5, "glowing embers"), ("campfire", 20, "a campfire"), ("big_fire", 100, "a big fire"), ("bonfire", 500, "a bonfire")]),
        Knob(key: "env.time_of_day", what: "the time of day", ru: "время", anchors: hours),
    ]

    /// Half-hour clock options with plain descriptions ("03:00" = 3 a.m., deep night).
    static let hours: [(String, Double, String)] = (0..<48).map { i in
        let h = Double(i) / 2
        let label = String(format: "%02d:%02d", i / 2, (i % 2) * 30)
        let part: String
        switch h {
        case ..<4: part = "deep night"
        case ..<5.5: part = "before dawn"
        case ..<7: part = "dawn, sunrise"
        case ..<11: part = "morning"
        case ..<13.5: part = "noon, midday"
        case ..<18: part = "afternoon"
        case ..<20: part = "evening"
        case ..<21.5: part = "dusk, sunset"
        default: part = "night"
        }
        let twelve = Int(h) % 12 == 0 ? 12 : Int(h) % 12
        return (label, h, "\(twelve)\(i % 2 == 1 ? ":30" : "") \(h < 12 ? "a.m." : "p.m."), \(part)")
    }
}

private extension String {
    var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
}
