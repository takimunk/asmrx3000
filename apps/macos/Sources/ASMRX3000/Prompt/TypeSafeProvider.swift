import Foundation

/// TypeSafe System One (Jev): POST https://api.typesafe.ai/v1/systemone. Jev answers typed
/// questions (choice / score / noul) with probabilities instead of writing text, so this
/// adapter asks one Choice per control in a single request ("fan-out"): keep it, move it a
/// little / plainly / a lot either way, or jump to a physical anchor ("gale", "03:00",
/// "small_waterfall"). Besides the controls it asks what kind of request it is (change the
/// scene, describe it, off topic), which topic a question is about, flight (fly / stand here / home), the
/// landscape (presets read from the state, never hardcoded), the pace, the side for birds
/// and what to listen to. Code turns the answers into the core's patch and writes the line
/// for the user itself (Jev writes no prose: answers about the scene come from the core's
/// description). Key: TYPESAFE_API_KEY or keychain account "typesafe".
struct TypeSafeProvider: PromptProvider {
    let name = "typesafe"
    let key: String
    var model = "jev-latest"
    static let endpoint = URL(string: "https://api.typesafe.ai/v1/systemone")!
    /// An answer whose winning option is less likely than this is ignored.
    static let minProbability = 0.4

    func complete(_ request: PromptRequest) async throws -> PromptReply {
        let scene = SceneState(state: request.state)
        let knobs = Knob.all(for: scene)
        var questions: [String: Any] = [:]
        let context = Self.context
        for k in knobs {
            questions[k.key] = ["type": "choice", "instructions": context + k.instructions, "criteria": k.criteria]
        }
        questions["intent"] = ["type": "choice", "instructions": context + Self.intentInstructions, "criteria": Self.intentOptions]
        questions["topic"] = ["type": "choice", "instructions": context + Self.topicInstructions, "criteria": Self.topicOptions]
        questions["glide"] = ["type": "choice", "instructions": context + Self.glideInstructions, "criteria": Self.glideOptions]
        questions["birds.side"] = ["type": "choice", "instructions": context + Self.sideInstructions, "criteria": Self.sideOptions]
        let focus = Self.focusOptions.filter { $0.key == "none" || scene.values["mix.focus.\($0.key)"] != nil }
        if focus.count > 1 {
            questions["focus"] = ["type": "choice", "instructions": context + Self.focusInstructions, "criteria": focus]
        }
        var stateBody: [String: Any] = ["request": request.text, "scene": scene.summary, "scene_now": request.summary]
        if let fb = request.feedback { stateBody["previous_attempt"] = fb }
        let body: [String: Any] = ["model": model, "state": stateBody, "questions": questions]
        let answers = try await post(body)
        if ProcessInfo.processInfo.environment["ASMR_PROMPT_DEBUG"] != nil {
            for (k, v) in answers.sorted(by: { $0.key < $1.key }) {
                guard let a = v as? [String: Any], let p = a["probabilities"] as? [String: Double] else { continue }
                let top = p.sorted { $0.value > $1.value }.prefix(3).map { "\($0.key) \(String(format: "%.2f", $0.value))" }
                print("  \(k): \(top.joined(separator: ", "))")
            }
        }
        return Self.reply(answers: answers, knobs: knobs, scene: scene, request: request)
    }

    /// Turns Jev's answers into a patch and the user's line. Pure: tested via --prompt-live.
    static func reply(answers: [String: Any], knobs: [Knob], scene: SceneState, request: PromptRequest) -> PromptReply {
        let russian = request.text.range(of: "\\p{Cyrillic}", options: .regularExpression) != nil
        let intent = pick(answers["intent"]) ?? "change"
        var changes: [[String: Any]] = []
        var said: [String] = []
        /// (has a specific phrase, text)
        var already: [(Bool, String)] = []
        var touched: Set<String> = []
        /// Groups whose physical amount the request is about ("убери дождь"): their loudness
        /// knob stays (otherwise "no rain" turns into "mute the rain sound").
        var physical: Set<String> = []

        func collect(maxUnchanged: Double, presetMax: Double) {
            for k in knobs where !touched.contains(k.key) && !(k.loudness && physical.contains(k.group)) {
                guard let current = k.key == "flight.on" ? Double(Knob.flightStates.firstIndex(of: scene.switches["flight.on"] ?? "off") ?? 0) : scene.values[k.key] else { continue }
                let target: Knob.Target?
                if k.key == "env.time_of_day" {
                    target = hour(answers[k.key], maxUnchanged: maxUnchanged).map { Knob.Target(value: $0, label: String(format: "%02d:%02d", Int($0), Int(($0 - $0.rounded(.down)) * 60)), relative: nil) }
                } else if let pick = change(answers[k.key], maxUnchanged: min(k.bar ?? 1, k.preset ? presetMax : maxUnchanged), minWinner: k.minWinner) {
                    target = k.target(pick, current)
                } else {
                    target = nil
                }
                guard let target else { continue }
                touched.insert(k.key)
                if k.amount { physical.insert(k.group) }
                // Already so: say it instead of "nothing found".
                let same = k.preset ? (k.key == "flight.on" ? target.label == (scene.switches["flight.on"] ?? "off") : scene.option(k.key) == target.label)
                    : abs(target.value - current) < 1e-3
                if same {
                    already.append((k.alreadyRu[target.label] != nil || k.key == "flight.biome", k.already(target, russian)))
                    continue
                }
                changes.append(["key": k.key, "value": k.preset ? target.label as Any : target.value])
                // The place leads the line.
                if k.key.hasPrefix("flight.on") || k.key == "flight.biome" {
                    said.insert(k.describe(target, current, russian), at: 0)
                } else {
                    said.append(k.describe(target, current, russian))
                }
            }
        }
        if intent != "off_topic" && intent != "describe" {
            collect(maxUnchanged: 0.5, presetMax: 0.25)
            // Nothing passed the bar although the request is a change: take the clearest
            // candidates once more with a lower bar rather than answering "nothing found".
            if changes.isEmpty && already.isEmpty && intent == "change" {
                collect(maxUnchanged: 0.7, presetMax: 0.5)
            }
        }

        // A landscape lives in flight mode: start flying, at a gentle pace if hovering.
        let keys = Set(changes.compactMap { $0["key"] as? String })
        let inWorld = (scene.switches["flight.on"] ?? "off") != "off"
        if keys.contains("flight.biome") && !inWorld && !keys.contains("flight.on") {
            changes.insert(["key": "flight.on", "value": "on"], at: 0)
        }
        // "Landing" together with a new landscape is a contradiction: a place means flying.
        if keys.contains("flight.biome"), let i = changes.firstIndex(where: { $0["key"] as? String == "flight.on" && $0["value"] as? String == "off" }) {
            changes[i]["value"] = "on"
            said.removeAll { $0 == "домой" || $0 == "home" }
        }
        let flyingAfter = keys.contains("flight.on") ? changes.contains { $0["key"] as? String == "flight.on" && $0["value"] as? String == "on" } : scene.switches["flight.on"] == "on" || (!inWorld && keys.contains("flight.biome"))
        if flyingAfter && !keys.contains("flight.speed") && (scene.values["flight.speed"] ?? 0) < 1 {
            changes.append(["key": "flight.speed", "value": 5.0])
        }

        if let f = pick(answers["focus"], min: 0.7), f != "none", intent != "describe", scene.values["mix.focus.\(f)"] != nil {
            changes.append(["key": "mix.focus.\(f)", "value": 0.8])
            said.append(russian ? "в фокусе \(focusRu[f] ?? f)" : "focus on \(f)")
        }
        var moves: [[String: Any]] = []
        if intent != "describe", let side = pick(answers["birds.side"]), side != "none", let tree = scene.nearestMovableTree(to: side) {
            moves.append(["object": tree.id, "position": tree.position])
            said.append(russian ? "птицы \(sideRu[side] ?? side)" : "birds \(side)")
        }
        let glide = glideSeconds[pick(answers["glide"]) ?? "normal"] ?? 4

        // The line for the user.
        let kind: PromptReply.Kind
        var explanation: String
        let answer = describe(topic: pick(answers["topic"]) ?? "all", request: request)
        if !said.isEmpty {
            kind = .patch
            let shown = said.prefix(5).joined(separator: ", ")
            let more = said.count > 5 ? (russian ? " и ещё \(said.count - 5)" : " and \(said.count - 5) more") : ""
            explanation = shown.capitalizedFirst + more + "."
            if intent == "both", !answer.isEmpty { explanation += " " + answer }
        } else if intent == "describe" || intent == "both" {
            kind = .answer
            explanation = answer.isEmpty ? (russian ? "Не могу описать сцену." : "Cannot describe the scene.") : answer
        } else if !already.isEmpty {
            kind = .nothing
            let best = already.filter(\.0).map(\.1)
            explanation = (best.isEmpty ? Array(already.prefix(1).map(\.1)) : Array(best.prefix(2))).joined(separator: ", ").capitalizedFirst + "."
        } else if intent == "off_topic" {
            kind = .nothing
            explanation = russian ? "Это не про звук. Я меняю воду, погоду, время, птиц, место и полёт." : "That is not about the sound scene."
        } else {
            kind = .unmatched
            let near = nearest(answers, knobs).map { russian ? " (ближе всего — \($0.ru))" : " (closest: \($0.en))" } ?? ""
            explanation = russian
                ? "Не понял, что изменить\(near). Скажите иначе: «тише река», «ночь», «в тайгу»."
                : "Nothing to change for that\(near)."
        }
        let patch: [String: Any] = ["explanation": explanation, "glide_s": glide, "changes": changes, "moves": moves]
        // Serialization of plain values cannot fail.
        return (try? PromptReply(patch: JSONSerialization.data(withJSONObject: patch), kind: kind))
            ?? (try! PromptReply(patch: Data("{\"explanation\":\"\",\"changes\":[]}".utf8), kind: .unmatched))
    }

    /// The answer to a question about the scene from the core's description.
    static func describe(topic: String, request: PromptRequest) -> String {
        let t = request.topics
        let pick: [String]
        switch topic {
        case "place": pick = ["place"]
        case "time": pick = ["time"]
        case "weather": pick = ["weather"]
        case "water": pick = ["water"]
        case "birds": pick = ["birds", "life"]
        case "loudest": pick = ["loudest"]
        default: pick = ["playing", "place", "time", "weather", "water", "birds"]
        }
        let parts = pick.compactMap { t[$0] }
        return parts.isEmpty ? request.summary : parts.joined(separator: " ")
    }

    /// The knob the model leaned to most, for "closest" in the nothing-found line.
    static func nearest(_ answers: [String: Any], _ knobs: [Knob]) -> Knob? {
        knobs.compactMap { k -> (Knob, Double)? in
            guard let a = answers[k.key] as? [String: Any], let p = a["probabilities"] as? [String: Double] else { return nil }
            return (k, p["unchanged"] ?? 1)
        }
        .filter { $0.1 < 0.9 }
        .min { $0.1 < $1.1 }?.0
    }

    /// The option to move to: only when "unchanged" is unlikely, then the likeliest other one.
    static func change(_ answer: Any?, maxUnchanged: Double, minWinner: Double = 0) -> String? {
        guard let a = answer as? [String: Any], let p = a["probabilities"] as? [String: Double] else { return nil }
        guard (p["unchanged"] ?? 0) < maxUnchanged else { return nil }
        guard let best = p.filter({ $0.key != "unchanged" }).max(by: { $0.value < $1.value }), best.value >= minWinner else { return nil }
        return best.key
    }

    /// Time of day: the probability spreads over neighbouring half hours, so take their
    /// circular mean (if "unchanged" is unlikely).
    static func hour(_ answer: Any?, maxUnchanged: Double = 0.5) -> Double? {
        guard let a = answer as? [String: Any], let p = a["probabilities"] as? [String: Double], (p["unchanged"] ?? 0) < maxUnchanged else { return nil }
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
        return (((h < 0 ? h + 24 : h) * 2).rounded() / 2).truncatingRemainder(dividingBy: 24)
    }

    /// Winning option of a choice answer if the model is sure enough.
    static func pick(_ answer: Any?, min: Double = minProbability) -> String? {
        guard let a = answer as? [String: Any], let choice = a["choice"] as? String else { return nil }
        let p = (a["probabilities"] as? [String: Any])?[choice] as? Double ?? 1
        return p >= min ? choice : nil
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
            if (code == 429 || code == 529 || code == 503) && attempt == 0 {
                try await Task.sleep(nanoseconds: 300_000_000)
                continue
            }
            guard code == 200 else {
                // The body names the failing field on 4xx; never contains the key.
                throw PromptError.http(code, String(decoding: data.prefix(400), as: UTF8.self))
            }
            guard let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let answers = o["answers"] as? [String: Any]
            else { throw PromptError.badReply("нет ответов на вопросы") }
            return answers
        }
        throw PromptError.http(429, "")
    }

    // MARK: Request kind, topic, pace, side, focus

    static let context = "The listener talks to a real-time soundscape (a forest by a river, or flight over generated landscapes). Their words are in `request` (often Russian), current values in `scene`, what is around now in `scene_now`. "

    static let intentInstructions = "What does the request want?"
    static let intentOptions: [String: Any] = [
        "change": "Change the sound scene: weather, time, water, birds, loudness, place, flying (e.g. 'тише река', 'хочу ночь', 'полетели', 'в пустыню', 'убери дождь').",
        "describe": "A question about what is around, what is heard, where things are; nothing to change (e.g. 'что вокруг?', 'где птицы?', 'что это шумит?', 'what's that sound').",
        "both": "Both: change something and describe it.",
        "off_topic": "Not about the soundscape at all (weather in a city, jokes, maths).",
    ]

    static let topicInstructions = "If the request asks about the scene, what is it about?"
    static let topicOptions: [String: Any] = [
        "all": "Everything around, the whole scene ('что вокруг?', 'опиши').",
        "place": "Where we are, the landscape, flying.",
        "time": "Time of day, season, temperature.",
        "weather": "Rain, thunder, wind.",
        "water": "The river, lake, sea, water.",
        "birds": "Birds, insects, animals: who sings and where.",
        "loudest": "What is loud, what that noise is.",
    ]

    static let glideInstructions = "How fast should the change happen?"
    static let glideOptions: [String: Any] = [
        "normal": "No pace is mentioned.",
        "gradual": "Slowly, gradually, fading, dying down, building up over time (e.g. 'rain is easing off', 'стихает', 'постепенно').",
        "instant": "Right now, suddenly, at once.",
    ]
    static let glideSeconds: [String: Double] = ["normal": 4, "gradual": 25, "instant": 0.8]

    static let sideInstructions = "Does the request say on which side of the listener the birds should be?"
    static let sideOptions: [String: Any] = [
        "none": "No side is mentioned for birds.",
        "left": "Birds on the left ('слева').",
        "right": "Birds on the right ('справа').",
        "ahead": "Birds in front ('впереди').",
        "behind": "Birds behind ('сзади').",
    ]
    static let sideRu = ["left": "слева", "right": "справа", "ahead": "впереди", "behind": "сзади"]

    static let focusInstructions = "Does the request say which sound to listen to, to bring to the foreground ('хочу слушать реку', 'focus on the birds', 'к морю' = the water)? Not about loudness alone."
    static let focusOptions: [String: Any] = [
        "none": "No sound to focus on.",
        "water": "The river, the sea, water.",
        "wind": "The wind and leaves.",
        "birds": "Birdsong.",
        "rain": "The rain and thunder.",
        "insects": "Crickets and insects.",
        "fire": "The campfire.",
    ]
    static let focusRu = ["water": "вода", "wind": "ветер", "birds": "птицы", "rain": "дождь", "insects": "насекомые", "fire": "костёр"]
}

/// One control (or switch) as a Choice: relative steps, physical anchors, or presets.
struct Knob {
    enum Step { case add(Double), times(Double) }
    /// States of the flight.on switch, by ASMR_PARAM_FLIGHT value.
    static let flightStates = ["off", "on", "stand"]

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
    /// Words for the explanation: up, down (Russian).
    var upDown: (String, String) = ("↑", "↓")
    /// "Already so" phrases by option or anchor name (Russian).
    var alreadyRu: [String: String] = [:]
    /// Stricter bar for this knob: it moves only if P(unchanged) is below this.
    var bar: Double?
    /// The winning option must be at least this likely (presets that tempt the model).
    var minWinner: Double = 0

    var group: String { String(key.split(separator: ".").first ?? "") }
    /// Loudness of one group (`<group>.level` / `.gain`), as opposed to its physical knobs.
    var loudness: Bool { key.hasSuffix(".level") || key.hasSuffix(".gain") }
    /// The physical amount of a group (its macro).
    var amount: Bool { ["water.flow", "rain.rate", "birds.activity", "wind.speed", "insects.activity", "fire.heat"].contains(key) }

    var preset: Bool { !presets.isEmpty }

    /// Short English label: the group ("wind", "birds"), or the parameter for env/storm/regime.
    var en: String {
        let parts = key.split(separator: ".").map(String.init)
        if key == "rain.storm" { return "thunder" }
        if parts[0] == "env" || key == "water.regime" || parts[0] == "flight" { return parts.last!.replacingOccurrences(of: "_", with: " ") }
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
        "How should \(what) (`scene.\(key)`) change to satisfy the request? Overall moods count: 'calmer' lowers wind, rain, river flow, birds and volume a little; " +
            "'night' means few birds and more insects; a storm brings rain and wind; a place ('пустыня', 'горы со снегом', 'море') implies its weather, temperature and life. " +
            "Pick `unchanged` if the request implies nothing about it."
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
        if key == "flight.on" {
            switch t.label {
            case "on": return russian ? "летим" : "flying"
            case "stand": return russian ? "стоим здесь" : "standing here"
            default: return russian ? "домой" : "home"
            }
        }
        if key == "flight.biome" { return russian ? Self.biomeRu(t.label) : t.label.replacingOccurrences(of: "_", with: " ") }
        if preset {
            let label = russian ? (Self.presetRu[t.label] ?? t.label.replacingOccurrences(of: "_", with: " ")) : t.label.replacingOccurrences(of: "_", with: " ")
            return key == "env.season" ? label : "\(name): \(label)"
        }
        if key == "env.time_of_day" { return t.label }
        let up = t.value > current
        if russian, upDown.0 != "↑" { return "\(name) \(up ? upDown.0 : upDown.1)" }
        return "\(name) " + (up ? "↑" : "↓")
    }

    func already(_ t: Target, _ russian: Bool) -> String {
        if let s = alreadyRu[t.label], russian { return s }
        if key == "flight.biome" { return russian ? "уже \(Self.biomeRu(t.label))" : "already \(t.label)" }
        return russian ? "\(ru): уже так" : "\(en) already"
    }

    static let presetRu = [
        "spring": "весна", "summer": "лето", "autumn": "осень", "winter": "зима",
        "scene": "как в сцене", "still_lake": "тихое озеро", "lake_lapping": "плеск озера", "slow_river": "медленная река",
        "brook_pebbles": "ручей по гальке", "brook_rocks": "ручей по камням", "rapids": "пороги", "small_waterfall": "водопад",
        "drips": "капель", "trickle": "струйка",
    ]

    static func biomeRu(_ name: String) -> String {
        ["mixed": "смешанный лес", "broadleaf": "лиственный лес", "taiga": "тайга", "river_valley": "речная долина",
         "meadows": "луга", "highlands": "нагорье", "desert": "пустыня", "snowy_mountains": "снежные горы",
         "lake_shore": "берег озера", "sea_coast": "морское побережье", "big_river": "большая река",
         "journey": "путешествие"][name] ?? name.replacingOccurrences(of: "_", with: " ")
    }

    /// What Jev reads about a landscape preset (English and the Russian words people use).
    static func biomeCriteria(_ name: String) -> String {
        [
            "mixed": "Mixed forest with brooks and lakes ('лес', 'смешанный лес').",
            "broadleaf": "Oak, birch and aspen woods ('дубрава', 'берёзовая роща', 'лиственный лес').",
            "taiga": "Spruce and pine taiga with lakes and rock ('тайга', 'хвойный лес', 'ельник').",
            "river_valley": "A valley with many brooks, willows and alders ('река', 'долина', 'ручьи').",
            "meadows": "Open meadows and fields with crickets ('луга', 'поле', 'степь').",
            "highlands": "Highlands: ridges, rock, waterfalls ('горы', 'скалы', 'нагорье').",
            "desert": "Desert: sand, dunes, heat, almost no trees ('пустыня', 'пески', 'дюны', 'Сахара').",
            "snowy_mountains": "Snowy mountains: snow, frost, peaks, cold wind ('снежные горы', 'в горы, где идёт снег', 'зимние горы').",
            "lake_shore": "A lake shore with lapping water ('озеро', 'на берегу озера', 'пруд').",
            "sea_coast": "A sea coast with surf ('море', 'к морю', 'побережье', 'океан', 'пляж').",
            "big_river": "A wide big river ('большая река', 'широкая река', 'на Волгу').",
            "journey": "A journey: the landscape changes as you fly, meadows, forests, lakes, taiga, mountains ('путешествие', 'разные места', 'покажи всё', 'лети куда-нибудь').",
        ][name] ?? "A landscape called '\(name.replacingOccurrences(of: "_", with: " "))'."
    }

    /// Knobs for the controls the state has: fixed knobs, loudness of each group (its
    /// `<group>.level` in dB when the core has one, else `<group>.gain`), flight and
    /// landscapes (the presets from the state).
    static func all(for scene: SceneState) -> [Knob] {
        var out = fixed.filter { scene.values[$0.key] != nil && ($0.key != "fire.heat" || scene.kinds.contains("fire")) }
        let loudness: [(String, String, String)] = [
            ("water", "the water", "вода"), ("rain", "the rain", "дождь"), ("birds", "the birds", "птицы"),
            ("wind", "the wind and leaves", "ветер"), ("insects", "the insects", "насекомые"), ("fire", "the campfire", "костёр"),
        ]
        for (g, what, ru) in loudness {
            let loudWhat = "how LOUD \(what) sounds (volume only; \(what) itself stays as it is: 'громче \(ru)', 'тише \(ru)', 'сделай \(ru) тише')"
            if scene.values["\(g).level"] != nil {
                let r = scene.ranges["\(g).level"] ?? -40...12
                out.append(Knob(key: "\(g).level", what: loudWhat, ru: ru, step: .add(4), range: r,
                                anchors: [("muted", r.lowerBound, "silent, muted"), ("far_background", -20, "far in the background"),
                                          ("as_calibrated", 0, "normal loudness")],
                                upDown: ("громче", "тише")))
            } else if scene.values["\(g).gain"] != nil, g != "fire" || scene.kinds.contains("fire") {
                out.append(Knob(key: "\(g).gain", what: loudWhat, ru: ru, step: .times(1.6), floor: 0.2, range: scene.ranges["\(g).gain"] ?? 0...4,
                                anchors: [("muted", 0, "silent, muted"), ("normal", 1, "normal loudness")],
                                upDown: ("громче", "тише")))
            }
        }
        if scene.switches["flight.on"] != nil {
            out.append(Knob(key: "flight.on", what: "where we are: flying through generated landscapes, standing still in one, or home in the forest by the river", ru: "полёт",
                            presets: [("on", "Start flying, fly on, or travel somewhere else: 'полетели', 'летим', 'дальше', 'перенеси меня в …', 'в пустыню', 'к морю', 'в горы'."),
                                      ("stand", "Stop right here and stand on the ground in this landscape: 'стоп', 'остановись', 'зависни', 'постоим', 'приземлись здесь'."),
                                      ("off", "Go home, back to the forest by the river: 'домой', 'приземлись домой', 'вернись домой'.")],
                            alreadyRu: ["on": "уже летим", "stand": "и так стоим", "off": "и так дома"]))
        }
        if let biomes = scene.options["flight.biome"], !biomes.isEmpty {
            out.append(Knob(key: "flight.biome", what: "the landscape to be in (a place request: desert, mountains, sea, lake, taiga…)", ru: "место",
                            presets: biomes.map { ($0, biomeCriteria($0)) }))
        }
        if scene.values["flight.speed"] != nil {
            out.append(Knob(key: "flight.speed", what: "the flight speed (only while flying: 'быстрее', 'медленнее'; 'стоп' / 'зависни' is flight.on stand)", ru: "скорость",
                            step: .times(1.6), floor: 2, range: 0...25,
                            anchors: [("hover", 0, "hover in place"), ("walk", 1.5, "walking pace"), ("glide", 8, "gentle glide"), ("fast", 20, "fast flight")],
                            upDown: ("быстрее", "медленнее")))
        }
        return out
    }

    static let fixed: [Knob] = [
        Knob(key: "water.flow", what: "how much water flows (the physical river: high water, a dry brook; NOT its loudness — 'тише река' / 'громче вода' alone are about loudness)", ru: "река",
             step: .add(0.1), anchors: [("dry", 0.05, "almost no flow"), ("low_brook", 0.35, "low summer brook"),
                                         ("as_built", 0.6, "normal"), ("high_water", 0.8, "high water"), ("flood", 1, "spring flood")],
             upDown: ("полноводнее", "мельче"), alreadyRu: ["dry": "река и так почти пересохла"]),
        Knob(key: "water.regime", what: "the character of the water", ru: "вода",
             presets: [("scene", "As the scene was built."), ("still_lake", "A still lake."), ("lake_lapping", "Small waves lapping a lake shore."),
                       ("slow_river", "A deep slow river."), ("brook_pebbles", "A babbling pebble brook."), ("brook_rocks", "A splashy rocky brook."),
                       ("rapids", "Roaring rapids, white water."), ("small_waterfall", "A small waterfall nearby."),
                       ("drips", "Cave-like water drips."), ("trickle", "A thin trickle.")], minWinner: 0.6),
        Knob(key: "wind.speed", what: "the wind", ru: "ветер", step: .times(1.35), floor: 1.5, range: 0...30,
             anchors: [("calm", 0.3, "dead calm"), ("light_air", 1.5, "light air"), ("gentle_breeze", 4, "gentle breeze"),
                       ("fresh_wind", 8, "fresh wind"), ("strong_wind", 12, "strong wind"), ("gale", 17, "gale"), ("storm_wind", 25, "storm wind")],
             upDown: ("сильнее", "слабее"), alreadyRu: ["calm": "и так безветрие"]),
        Knob(key: "rain.rate", what: "how much it rains (snow is not rain: snowfall means winter, keep rain unchanged or dry)", ru: "дождь", step: .times(1.8), floor: 0.6, range: 0...100,
             anchors: [("dry", 0, "no rain, stop the rain"), ("drizzle", 0.3, "drizzle"), ("light_rain", 1, "light rain"), ("steady_rain", 4, "steady rain"),
                       ("heavy_rain", 10, "heavy rain"), ("downpour", 25, "downpour"), ("cloudburst", 60, "cloudburst")],
             upDown: ("сильнее", "слабее"), alreadyRu: ["dry": "дождя и так нет"]),
        Knob(key: "rain.storm", what: "thunder and lightning", ru: "гроза", step: .add(0.2),
             anchors: [("no_thunder", 0, "no thunder"), ("distant_rumbles", 0.2, "distant rumbles"),
                       ("storm_nearby", 0.6, "thunderstorm nearby"), ("overhead", 0.9, "storm right overhead")],
             alreadyRu: ["no_thunder": "грозы и так нет"]),
        Knob(key: "birds.activity", what: "how many birds sing (the number of birds, not their loudness)", ru: "птицы", step: .add(0.15),
             anchors: [("silent", 0, "no birdsong"), ("few_birds", 0.25, "a few birds"), ("normal", 0.6, "normal"), ("full_chorus", 1, "full chorus")],
             upDown: ("больше", "меньше"), alreadyRu: ["silent": "птиц и так нет", "full_chorus": "птиц и так полный хор"]),
        Knob(key: "insects.activity", what: "how many crickets and other insects there are", ru: "насекомые", step: .times(1.4), floor: 0.5, range: 0...2,
             anchors: [("silent", 0, "no insects"), ("normal", 1, "as in the scene"), ("loud", 2, "loud crickets")],
             upDown: ("больше", "меньше"), alreadyRu: ["silent": "насекомых и так нет"]),
        Knob(key: "master.volume", what: "the overall volume of everything (only when the request is about all sound, not one thing)", ru: "громкость", step: .add(0.07), range: 0...1.5, upDown: ("выше", "ниже"), bar: 0.25),
        Knob(key: "env.temperature", what: "the air temperature", ru: "температура", step: .add(3), range: -20...40,
             anchors: [("freezing", -5, "freezing"), ("cold", 5, "cold"), ("cool", 12, "cool"), ("mild", 18, "mild"), ("warm", 24, "warm"), ("hot", 32, "hot")],
             upDown: ("теплее", "холоднее")),
        Knob(key: "env.season", what: "the season (snow = winter)", ru: "сезон",
             presets: [("spring", "Spring."), ("summer", "Summer."), ("autumn", "Autumn, fall."), ("winter", "Winter, snow.")]),
        Knob(key: "fire.heat", what: "the campfire", ru: "костёр", step: .times(1.8), floor: 5, range: 1...1000,
             anchors: [("embers", 5, "glowing embers"), ("campfire", 20, "a campfire"), ("big_fire", 100, "a big fire"), ("bonfire", 500, "a bonfire")],
             upDown: ("жарче", "тише")),
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

extension String {
    var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
}
