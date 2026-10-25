// l10n-ok-file: Russian here is LLM prompt text and the offline parser's keywords, not UI.
import Foundation
import Security

// Prompt control: provider-agnostic interface. The core builds everything model-facing
// (asmr_get_prompt_json: system prompt, current state, tool schema, the scene in words); a
// provider only carries it to a model and brings back the tool input, which is a patch for
// asmr_apply_patch_json. See docs/prompt-control.md.

/// What a provider sends.
struct PromptRequest {
    /// The user's words.
    var text: String
    /// Static system prompt (rules, what each control means, few-shot examples). Cacheable.
    var system: String
    /// Current values as text, sent alongside the request.
    var context: String
    /// The same state as JSON (asmr_get_state_json), for providers that take structured state.
    var state: Data
    var toolName: String
    var toolDescription: String
    /// JSON schema of the tool input, generated from the control tree.
    var schema: Data
    /// "What is around" in Russian (one paragraph) and by topic (place, time, weather, water,
    /// birds, life, loudest), built by the core from the live engine.
    var summary: String
    var topics: [String: String]
    /// Set on a retry: what went wrong with the previous reply, in words for the model.
    var feedback: String?

    init(text: String, promptJSON: Data, state: Data) throws {
        guard let o = try JSONSerialization.jsonObject(with: promptJSON) as? [String: Any],
              let tool = o["tool"] as? [String: Any],
              let schema = tool["input_schema"]
        else { throw PromptError.badReply(tr("the core gave no prompt")) }
        self.text = text
        system = o["system"] as? String ?? ""
        context = o["context"] as? String ?? ""
        self.state = state
        toolName = tool["name"] as? String ?? "set_scene"
        toolDescription = tool["description"] as? String ?? ""
        self.schema = try JSONSerialization.data(withJSONObject: schema)
        summary = o["summary_ru"] as? String ?? ""
        topics = o["summary_topics_ru"] as? [String: String] ?? [:]
    }
}

/// What a provider returns.
struct PromptReply {
    enum Kind: Equatable {
        /// A patch with changes (it may still fail in the core).
        case patch
        /// An answer about the scene; nothing to change.
        case answer
        /// The request is already satisfied, or not about the scene: nothing to change, by design.
        case nothing
        /// The model found nothing to change and could not say why.
        case unmatched
    }

    /// Tool input JSON: {"explanation", "glide_s"?, "changes": [...], "moves"?: [...]}.
    var patch: Data
    var explanation: String
    var kind: Kind

    init(patch: Data, kind: Kind? = nil) throws {
        guard let o = try? JSONSerialization.jsonObject(with: patch) as? [String: Any] else {
            throw PromptError.badReply(tr("the reply is not a JSON object"))
        }
        self.patch = patch
        explanation = (o["explanation"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let empty = ((o["changes"] as? [Any])?.isEmpty ?? true) && ((o["moves"] as? [Any])?.isEmpty ?? true)
        self.kind = kind ?? (empty ? .nothing : .patch)
    }
}

protocol PromptProvider {
    /// Short name for the UI and logs.
    var name: String { get }
    /// Whether the provider writes keys itself (an LLM) and so can fix a reply when told what
    /// the core rejected (`PromptRequest.feedback`). Typed-question providers cannot.
    var repairs: Bool { get }
    func complete(_ request: PromptRequest) async throws -> PromptReply
}

extension PromptProvider {
    var repairs: Bool { false }
}

enum PromptError: LocalizedError {
    case noProvider
    case noKey(String)
    case offline
    case timeout
    case http(Int, String)
    case badReply(String)
    /// The core applied nothing: its own reasons.
    case rejected([String])

    /// One line for the popover: what failed, precisely, in the current language.
    var errorDescription: String? {
        switch self {
        case .noProvider:
            return tr("No model connected: no TypeSafe key (see docs/prompt-control.md). The sliders work.")
        case let .noKey(service):
            return tr("No %@ key.", service)
        case .offline:
            return tr("No network: the model is unreachable. The sliders work.")
        case .timeout:
            return tr("The model did not answer in %d s. Try again.", Int(PromptController.timeout))
        case let .http(code, body):
            let detail = Self.detail(body)
            switch code {
            case 401, 403: return tr("Key rejected (%d): check the key in the keychain.", code)
            case 429: return tr("The model is overloaded or out of quota (429). Try later.")
            case 400, 422: return tr("The model rejected the request (%d)", code) + (detail.isEmpty ? "." : ": \(detail)")
            case 500...599: return tr("The model service is failing (%d). Try later.", code)
            default: return tr("Service error (%d)", code) + (detail.isEmpty ? "." : ": \(detail)")
            }
        case let .badReply(why):
            return tr("The model replied with something unreadable: %@.", why)
        case let .rejected(reasons):
            return tr("Not applied: %@.", reasons.prefix(2).joined(separator: "; "))
        }
    }

    /// The service's own message from an error body ({"error": {"message": …}}, {"detail": …}).
    static func detail(_ body: String) -> String {
        guard let d = body.data(using: .utf8), let o = try? JSONSerialization.jsonObject(with: d) else {
            return String(body.prefix(80))
        }
        func find(_ x: Any) -> String? {
            if let s = x as? String { return s }
            if let o = x as? [String: Any] {
                for k in ["message", "detail", "error", "msg"] { if let v = o[k], let s = find(v) { return s } }
            }
            if let a = x as? [Any], let first = a.first { return find(first) }
            return nil
        }
        return String((find(o) ?? "").prefix(100))
    }

    /// Maps transport errors to the few cases the UI distinguishes.
    static func from(_ error: Error) -> PromptError {
        if let e = error as? PromptError { return e }
        if let u = error as? URLError {
            switch u.code {
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed, .dataNotAllowed:
                return .offline
            case .timedOut:
                return .timeout
            default:
                return .badReply(tr("network: %@", u.localizedDescription))
            }
        }
        return .badReply(error.localizedDescription)
    }
}

/// API keys: the environment first (handy from a terminal), then the login keychain
/// (generic password), never in files or logs. The user adds one in the onboarding's last step
/// or the panel's settings, or with
///     security add-generic-password -U -s com.asmrx3000.prompt -a <provider> -w
enum Keychain {
    static let service = "com.asmrx3000.prompt"

    /// The accounts the app knows, with their environment variables.
    enum Account: String {
        case typesafe, fal

        var env: String {
            switch self {
            case .typesafe: return "TYPESAFE_API_KEY"
            case .fal: return "FAL_KEY"
            }
        }
    }

    /// A key from the environment.
    static func env(_ name: String) -> String? {
        guard let v = ProcessInfo.processInfo.environment[name]?.trimmingCharacters(in: .whitespacesAndNewlines), !v.isEmpty else { return nil }
        return v
    }

    static func key(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let key = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !key.isEmpty
        else { return nil }
        return key
    }

    // Read once per run (the first keychain read of an item the app did not write may show the
    // "allow" dialog), and replaced when the app saves or removes a key, so a key added in the
    // settings works at once.
    private static let lock = NSLock()
    private static var cache: [Account: String?] = [:]

    /// The key in use for `account`: the environment, else the keychain.
    static func stored(_ account: Account) -> String? {
        lock.lock()
        if let hit = cache[account] { lock.unlock(); return hit }
        lock.unlock()
        let v = env(account.env) ?? key(account: account.rawValue)
        lock.lock(); cache[account] = .some(v); lock.unlock()
        return v
    }

    /// Whether a key is set, without reading the secret (so no keychain dialog): the
    /// environment, a key read or saved this run, or an item in the keychain.
    static func has(_ account: Account) -> Bool {
        if env(account.env) != nil { return true }
        lock.lock()
        let hit = cache[account]
        lock.unlock()
        if let hit { return hit != nil }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        return SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess
    }

    /// Whether the key comes from the environment (then the keychain item is not used).
    static func fromEnvironment(_ account: Account) -> Bool { env(account.env) != nil }

    /// Saves (or replaces) the key in the login keychain. True on success.
    @discardableResult
    static func save(_ value: String, account: Account) -> Bool {
        let key = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty, let data = key.data(using: .utf8) else { return false }
        let match: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue,
        ]
        var status = SecItemUpdate(match as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var add = match
            add[kSecValueData as String] = data
            add[kSecAttrLabel as String] = "ASMR X3000 (\(account.rawValue))"
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            NSLog("ASMR X3000: keychain save for %@ failed: %d", account.rawValue, status)
            return false
        }
        lock.lock(); cache[account] = .some(env(account.env) ?? key); lock.unlock()
        return true
    }

    /// Removes the key from the keychain (the environment, if set, still wins).
    static func remove(_ account: Account) {
        let match: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue,
        ]
        SecItemDelete(match as CFDictionary)
        lock.lock(); cache[account] = .some(env(account.env)); lock.unlock()
    }

    /// A light check of what was pasted: no spaces, long enough, and for fal the "id:secret"
    /// shape. Returns a short reason when it does not look like a key.
    static func problem(_ value: String, account: Account) -> String? {
        let v = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if v.isEmpty { return "empty" }
        if v.rangeOfCharacter(from: .whitespacesAndNewlines) != nil { return "has spaces" }
        if v.count < 16 { return "too short" }
        if account == .fal, !v.contains(":") { return "a fal key looks like id:secret" }
        return nil
    }
}

/// Picks the provider: `ASMR_PROMPT_PROVIDER=mock` forces the offline mock (snapshots,
/// testing without a key); otherwise the first real adapter that has a key.
enum Providers {
    static func current() -> PromptProvider? {
        if ProcessInfo.processInfo.environment["ASMR_PROMPT_PROVIDER"] == "mock" {
            return MockProvider()
        }
        if let key = typesafeKey {
            return TypeSafeProvider(key: key)
        }
        return nil
    }

    /// Cached by Keychain (read once per run, replaced when a key is saved in the app).
    private static var typesafeKey: String? { Keychain.stored(.typesafe) }
}

/// Offline stand-in: a handful of keyword rules that produce the same kind of patch a model
/// would, from the current state. Used by --snapshot and when ASMR_PROMPT_PROVIDER=mock.
struct MockProvider: PromptProvider {
    let name = "mock"

    func complete(_ request: PromptRequest) async throws -> PromptReply {
        try reply(request)
    }

    func reply(_ request: PromptRequest) throws -> PromptReply {
        let scene = SceneState(state: request.state)
        let values = scene.values
        let t = request.text.lowercased()
        func has(_ words: String...) -> Bool { words.contains { t.contains($0) } }
        var changes: [[String: Any]] = []
        var said: [String] = []
        var glide = 4.0
        func set(_ key: String, _ value: Any) { changes.append(["key": key, "value": value]) }
        func scaled(_ key: String, _ f: Double) { set(key, (values[key] ?? 0) * f) }

        if has("что вокруг", "что слышно", "где птиц", "опиши", "what's around", "describe") {
            let answer = request.summary.isEmpty ? tr("I do not know what is around.") : request.summary
            let patch: [String: Any] = ["explanation": answer, "changes": []]
            return try PromptReply(patch: JSONSerialization.data(withJSONObject: patch), kind: .answer)
        }
        let loud = has("громче", "louder"), quiet = has("тише", "quieter")
        let water = has("вод", "рек", "river", "water")
        if (loud || quiet) && water {
            if scene.values["water.level"] != nil { set("water.level", loud ? 6 : -8) } else { scaled("water.gain", loud ? 2 : 0.4) }
            said.append(loud ? tr("water louder") : tr("water quieter"))
        } else if has("спокой", "тише", "calm", "quiet") {
            scaled("wind.speed", 0.55); scaled("rain.rate", 0.4); scaled("water.flow", 0.8)
            scaled("birds.activity", 0.65); scaled("master.volume", 0.92)
            said.append(tr("calmer"))
        }
        if has("ноч", "night") { set("env.time_of_day", 23.5); set("birds.activity", 0.25); said.append(tr("night")) }
        if has("утр", "рассвет", "dawn", "morning") { set("env.time_of_day", 5.5); set("birds.activity", 1); said.append(tr("dawn")) }
        if has("убери дожд", "без дожд", "no rain") {
            if (values["rain.rate"] ?? 0) < 0.05 {
                let patch: [String: Any] = ["explanation": tr("There is no rain anyway."), "changes": []]
                return try PromptReply(patch: JSONSerialization.data(withJSONObject: patch), kind: .nothing)
            }
            set("rain.rate", 0); set("rain.storm", 0); said.append(tr("no rain"))
        } else if has("гроз", "storm") {
            set("rain.rate", 25); set("rain.storm", 0.8); set("wind.speed", 11); said.append(tr("thunderstorm"))
        } else if has("дожд", "rain") {
            set("rain.rate", has("лёгк", "легк", "light", "мелк") ? 1 : 6); said.append(tr("rain"))
        }
        if has("снег", "зим", "snow") { set("env.season", "winter"); set("env.temperature", -6); said.append(tr("winter")) }
        if has("стиха", "fading", "постепенно", "slowly") { glide = 25; if has("дожд", "rain") { set("rain.rate", 0.2) } }
        if has("водопад", "waterfall") {
            set("water.regime", "small_waterfall"); set("water.flow", 0.9); set("water.aeration", 1.3); said.append(tr("waterfall"))
        }
        if has("больше птиц", "more birds") { set("birds.activity", min(1, (values["birds.activity"] ?? 0.6) + 0.3)); said.append(tr("more birds")) }
        if loud && !water { scaled("master.volume", 1.2); said.append(tr("louder")) }
        // Where it blows from (compound sides first).
        let sides: [(words: [String], deg: Double, name: String)] = [
            (["северо-запад", "northwest", "north-west"], 315, N_("from the northwest")), (["северо-восток", "northeast", "north-east"], 45, N_("from the northeast")),
            (["юго-запад", "southwest", "south-west"], 225, N_("from the southwest")), (["юго-восток", "southeast", "south-east"], 135, N_("from the southeast")),
            (["с севера", "северн", "from the north", "northerly"], 0, N_("from the north")), (["с юга", "южн", "from the south", "southerly"], 180, N_("from the south")),
            (["с запада", "западн", "from the west", "westerly"], 270, N_("from the west")), (["с востока", "восточн", "from the east", "easterly"], 90, N_("from the east")),
        ]
        let side = sides.first(where: { s in s.words.contains { t.contains($0) } })
        // A side alone ("ветер с севера") turns the wind; it gets stronger only when asked.
        if has("ветер", "ветр", "wind") && !has("гроз", "storm") && (side == nil || has("сильн", "strong", "ветрен", "windy")) {
            set("wind.speed", 9); said.append(tr("wind"))
        }
        if let side {
            set("env.wind_direction", side.deg); said.append(tr("wind %@", tr(side.name)))
        }
        let explanation = said.isEmpty ? "" : tr("Mock: %@.", said.joined(separator: ", "))
        let patch: [String: Any] = ["explanation": explanation, "glide_s": glide, "changes": changes]
        return try PromptReply(patch: JSONSerialization.data(withJSONObject: patch), kind: said.isEmpty ? .unmatched : .patch)
    }
}

/// Current values, options and objects from asmr_get_state_json.
struct SceneState {
    var values: [String: Double] = [:]
    var units: [String: String] = [:]
    var options: [String: [String]] = [:]
    var ranges: [String: ClosedRange<Double>] = [:]
    /// `<group>.level` controls (dB loudness of one group).
    var loudness: Set<String> = []
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
            if let lo = c["min"] as? Double, let hi = c["max"] as? Double, lo <= hi { ranges[k] = lo...hi }
            if c["loudness"] as? Bool == true { loudness.insert(k) }
        }
        for obj in o["objects"] as? [[String: Any]] ?? [] {
            if let k = obj["kind"] as? String { kinds.insert(k) }
            if obj["kind"] as? String == "tree", obj["movable"] as? Bool == true,
               let id = obj["id"] as? Int, let p = obj["pos"] as? [Double], p.count == 3 {
                trees.append((id, p))
            }
        }
    }

    /// Plain "key: value unit" map a model reads as state.
    var summary: [String: String] {
        var s: [String: String] = [:]
        for (k, v) in values {
            if let opts = options[k], Int(v.rounded()) < opts.count, v >= 0 {
                s[k] = opts[Int(v.rounded())]
            } else {
                s[k] = String(format: "%g", (v * 100).rounded() / 100) + ((units[k] ?? "").isEmpty ? "" : " " + units[k]!)
            }
        }
        return s
    }

    func option(_ key: String) -> String? {
        guard let v = values[key], let opts = options[key], v >= 0, Int(v.rounded()) < opts.count else { return nil }
        return opts[Int(v.rounded())]
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

/// `ASMRX3000 --prompt-live "…" ["…" …]`: real requests through the configured provider (or
/// the mock with ASMR_PROMPT_PROVIDER=mock); prints the patch, the core's result, what the
/// prompt line would say and the latency (never the key). `--describe` prints the scene in words.
enum PromptCheck {
    static func run(_ texts: [String]) {
        let audio = AudioEngine()
        if texts == ["--describe"] {
            audio.simulate(seconds: 3)
            let request = try? PromptRequest(text: "", promptJSON: audio.promptJSON(), state: audio.stateJSON())
            print(request?.summary ?? "no summary")
            return
        }
        guard let provider = Providers.current() else {
            print("no provider: add a key with `security add-generic-password -U -s \(Keychain.service) -a typesafe -w` or set TYPESAFE_API_KEY")
            return
        }
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            for text in texts {
                print("=== \(text)")
                do {
                    let request = try PromptRequest(text: text, promptJSON: audio.promptJSON(), state: audio.stateJSON())
                    let start = Date()
                    let reply = try await provider.complete(request)
                    let ms = Int(Date().timeIntervalSince(start) * 1000)
                    print("provider \(provider.name), \(ms) ms, kind \(reply.kind)")
                    print("patch  " + String(decoding: reply.patch, as: UTF8.self))
                    let result = audio.applyPatch(reply.patch)
                    print("result ok=\(result["ok"] ?? "?") errors=\((result["errors_ru"] as? [String]) ?? [])")
                    print("line   \(PromptController.line(for: reply, result: result).text)")
                } catch {
                    print("failed: \(PromptError.from(error).errorDescription ?? "") (\(error))")
                }
            }
            done.signal()
        }
        done.wait()
    }
}
