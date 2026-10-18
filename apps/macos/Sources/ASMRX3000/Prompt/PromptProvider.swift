import Foundation
import Security

// Prompt control: provider-agnostic interface. The core builds everything model-facing
// (asmr_get_prompt_json: system prompt, current state, tool schema); a provider only carries
// it to a model and brings back the tool input, which is a patch for asmr_apply_patch_json.
// See docs/prompt-control.md.

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

    init(text: String, promptJSON: Data, state: Data) throws {
        guard let o = try JSONSerialization.jsonObject(with: promptJSON) as? [String: Any],
              let tool = o["tool"] as? [String: Any],
              let schema = tool["input_schema"]
        else { throw PromptError.badReply("core returned no prompt") }
        self.text = text
        system = o["system"] as? String ?? ""
        context = o["context"] as? String ?? ""
        self.state = state
        toolName = tool["name"] as? String ?? "set_scene"
        toolDescription = tool["description"] as? String ?? ""
        self.schema = try JSONSerialization.data(withJSONObject: schema)
    }
}

/// What a provider returns.
struct PromptReply {
    /// Tool input JSON: {"explanation", "glide_s"?, "changes": [...], "moves"?: [...]}.
    var patch: Data
    var explanation: String

    init(patch: Data) throws {
        guard let o = try? JSONSerialization.jsonObject(with: patch) as? [String: Any] else {
            throw PromptError.badReply("reply is not a JSON object")
        }
        self.patch = patch
        explanation = (o["explanation"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

protocol PromptProvider {
    /// Short name for the UI and logs.
    var name: String { get }
    func complete(_ request: PromptRequest) async throws -> PromptReply
}

enum PromptError: LocalizedError {
    case noProvider
    case noKey
    case offline
    case timeout
    case http(Int, String)
    case badReply(String)

    /// One quiet line for the popover.
    var errorDescription: String? {
        switch self {
        case .noProvider: return "Модель не подключена — ползунки работают как прежде."
        case .noKey: return "Нет ключа API в связке ключей."
        case .offline: return "Нет сети — ползунки работают как прежде."
        case .timeout: return "Модель не ответила вовремя."
        case let .http(code, _): return code == 401 || code == 403 ? "Ключ API не принят." : "Ошибка сервиса (\(code))."
        case .badReply: return "Модель ответила непонятно."
        }
    }

    /// Maps transport errors to the few cases the UI distinguishes.
    static func from(_ error: Error) -> PromptError {
        if let e = error as? PromptError { return e }
        if let u = error as? URLError {
            switch u.code {
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed:
                return .offline
            case .timedOut:
                return .timeout
            default:
                break
            }
        }
        return .badReply(error.localizedDescription)
    }
}

/// API keys live in the login keychain (generic password), never in files or logs. The user
/// adds one with
///     security add-generic-password -U -s com.asmrx3000.prompt -a <provider> -w
enum Keychain {
    static let service = "com.asmrx3000.prompt"

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
}

/// Picks the provider: `ASMR_PROMPT_PROVIDER=mock` forces the offline mock (snapshots,
/// testing without a key); otherwise the first real adapter that has a key.
enum Providers {
    static func current() -> PromptProvider? {
        if ProcessInfo.processInfo.environment["ASMR_PROMPT_PROVIDER"] == "mock" {
            return MockProvider()
        }
        // Real adapters go here (TypeSafe / System One: pending its API docs).
        return nil
    }
}

/// Offline stand-in: a handful of keyword rules that produce the same kind of patch a model
/// would, from the current state. Used by --snapshot and when ASMR_PROMPT_PROVIDER=mock.
struct MockProvider: PromptProvider {
    let name = "mock"

    func complete(_ request: PromptRequest) async throws -> PromptReply {
        try reply(request)
    }

    func reply(_ request: PromptRequest) throws -> PromptReply {
        var values: [String: Double] = [:]
        if let o = try? JSONSerialization.jsonObject(with: request.state) as? [String: Any],
           let controls = o["controls"] as? [[String: Any]] {
            for c in controls {
                if let k = c["key"] as? String, let v = c["value"] as? Double { values[k] = v }
            }
        }
        let t = request.text.lowercased()
        func has(_ words: String...) -> Bool { words.contains { t.contains($0) } }
        var changes: [[String: Any]] = []
        var said: [String] = []
        var glide = 4.0
        func set(_ key: String, _ value: Any) { changes.append(["key": key, "value": value]) }
        func scaled(_ key: String, _ f: Double) { set(key, (values[key] ?? 0) * f) }

        if has("спокой", "тише", "calm", "quiet") {
            scaled("wind.speed", 0.55); scaled("rain.rate", 0.4); scaled("water.flow", 0.8)
            scaled("birds.activity", 0.65); scaled("master.volume", 0.92)
            said.append("спокойнее")
        }
        if has("ноч", "night") { set("env.time_of_day", 23.5); set("birds.activity", 0.25); said.append("ночь") }
        if has("утр", "рассвет", "dawn", "morning") { set("env.time_of_day", 5.5); set("birds.activity", 1); said.append("рассвет") }
        if has("гроз", "storm") {
            set("rain.rate", 25); set("rain.storm", 0.8); set("wind.speed", 11); said.append("гроза")
        } else if has("дожд", "rain") {
            set("rain.rate", has("лёгк", "легк", "light", "мелк") ? 1 : 6); said.append("дождь")
        }
        if has("стиха", "fading", "постепенно", "slowly") { glide = 25; if has("дожд", "rain") { set("rain.rate", 0.2) } }
        if has("водопад", "waterfall") {
            set("water.regime", "small_waterfall"); set("water.flow", 0.9); set("water.aeration", 1.3); said.append("водопад")
        }
        if has("больше птиц", "more birds") { set("birds.activity", min(1, (values["birds.activity"] ?? 0.6) + 0.3)); said.append("больше птиц") }
        if has("громче", "louder") { scaled("master.volume", 1.2); said.append("громче") }
        if has("ветер", "ветр", "wind") && !has("гроз", "storm") { set("wind.speed", 9); said.append("ветер") }

        let explanation = said.isEmpty ? "Не понял, что изменить." : "Мок: " + said.joined(separator: ", ") + "."
        let patch: [String: Any] = ["explanation": explanation, "glide_s": glide, "changes": changes]
        return try PromptReply(patch: JSONSerialization.data(withJSONObject: patch))
    }
}
