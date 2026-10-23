import Combine
import Foundation

/// The app's languages. English is the base and the release language; Russian is a full
/// localization. The choice is the user's, made in the first onboarding step and changeable in
/// the panel's settings; it applies live (see `Language`), independent of the macOS language.
enum Lang: String, CaseIterable, Identifiable {
    case en, ru
    var id: String { rawValue }
    /// The language's own name, as the picker shows it.
    var native: String { self == .en ? "English" : "Русский" } // l10n-ok
    /// The English name, for telling an LLM which language to answer in.
    var englishName: String { self == .en ? "English" : "Russian" }
    var locale: Locale { Locale(identifier: rawValue) }
}

/// The chosen language. Views that show text hold `@ObservedObject var lang = Language.shared`
/// so they re-render on a change; AppKit pieces (menus, window titles) subscribe to `$current`.
/// Stored in UserDefaults ("language"); English until the user picks otherwise. `--lang ru`
/// (or ASMR_LANG=ru) overrides it for one run without storing it, for snapshots and tests.
final class Language: ObservableObject {
    static let shared = Language()
    static let key = "language"

    @Published private(set) var current: Lang

    private init() {
        let args = CommandLine.arguments
        let forced = args.firstIndex(of: "--lang").flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
            ?? ProcessInfo.processInfo.environment["ASMR_LANG"]
        if let forced, let l = Lang(rawValue: forced) {
            current = l
        } else {
            current = UserDefaults.standard.string(forKey: Self.key).flatMap(Lang.init(rawValue:)) ?? .en
        }
        L10n.set(current)
    }

    /// Switches the language everywhere: strings looked up from now on, and every observer.
    func select(_ lang: Lang) {
        guard lang != current else { return }
        UserDefaults.standard.set(lang.rawValue, forKey: Self.key)
        L10n.set(lang)
        current = lang
    }

    /// Has the user ever chosen (the onboarding's first step has been answered)?
    static var chosen: Bool { UserDefaults.standard.string(forKey: key) != nil }
}

/// String lookup. Keys are the English source strings; the `.strings` files in
/// Resources/Localization/<lang>.lproj hold the translations (every file in the folder is read
/// into one table, so the files are split by area only for the editor's sake). English needs no
/// file (the key is the text) except for plural forms (en.lproj/Plurals.strings); a missing
/// Russian string falls back to the key, i.e. English. scripts/check-l10n.py keeps the call
/// sites and the files in step.
///
/// This is not NSLocalizedString on purpose: the language is the app's own setting, applied
/// live, and SwiftPM's `Bundle.module` does not resolve inside a hand-assembled .app.
enum L10n {
    private static let lock = NSLock()
    private static var language = Lang.en
    private static var tables: [Lang: [String: String]] = [:]

    static var current: Lang {
        lock.lock(); defer { lock.unlock() }
        return language
    }

    static var locale: Locale { current.locale }

    fileprivate static func set(_ lang: Lang) {
        lock.lock(); defer { lock.unlock() }
        language = lang
        if tables[lang] == nil { tables[lang] = load(lang) }
    }

    /// Resources/Localization inside the app; the source tree for runs from .build.
    static let directory: URL? = {
        if let r = Bundle.main.resourceURL?.appendingPathComponent("Localization"),
           FileManager.default.fileExists(atPath: r.path) {
            return r
        }
        let dev = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../Resources/Localization").standardizedFileURL
        return FileManager.default.fileExists(atPath: dev.path) ? dev : nil
    }()

    private static func load(_ lang: Lang) -> [String: String] {
        guard let dir = directory?.appendingPathComponent(lang.rawValue + ".lproj"),
              let files = try? FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil) else { return [:] }
        var table: [String: String] = [:]
        for f in files where f.pathExtension == "strings" {
            if let d = NSDictionary(contentsOf: f) as? [String: String] { table.merge(d) { a, _ in a } }
        }
        return table
    }

    /// The string for `key` in the current language.
    static func t(_ key: String) -> String {
        _ = booted
        lock.lock(); defer { lock.unlock() }
        return tables[language]?[key] ?? key
    }

    /// Makes sure the stored language is read before the first look-up.
    private static let booted: Bool = { _ = Language.shared; return true }()

    /// `key` as a format string: `tr("set by %@", name)`. Formatted in the current language's
    /// locale (decimal comma in Russian).
    static func t(_ key: String, _ args: [CVarArg]) -> String {
        String(format: t(key), locale: locale, arguments: args)
    }

    /// The string for a count, with the language's plural forms: keys `key#one`, `key#few`,
    /// `key#many`, `key#other` (Russian uses all four, English `one` and `other`; any missing
    /// form falls back to `#other`). The count is the format's only argument.
    static func plural(_ key: String, _ n: Int) -> String {
        _ = booted
        let form = pluralForm(n)
        lock.lock()
        let table = tables[language] ?? [:]
        lock.unlock()
        let format = table["\(key)#\(form)"] ?? table["\(key)#other"] ?? key
        return String(format: format, locale: locale, n)
    }

    private static func pluralForm(_ n: Int) -> String {
        let n = abs(n)
        switch current {
        case .en: return n == 1 ? "one" : "other"
        case .ru:
            if n % 10 == 1 && n % 100 != 11 { return "one" }
            if (2...4).contains(n % 10) && !(12...14).contains(n % 100) { return "few" }
            return "many"
        }
    }

    /// A number in the current language's format ("0.5" / "0,5"), at most `digits` decimals.
    static func number(_ v: Double, digits: Int = 0) -> String {
        let f = NumberFormatter()
        f.locale = locale
        f.numberStyle = .decimal
        f.minimumFractionDigits = 0
        f.maximumFractionDigits = digits
        f.usesGroupingSeparator = false
        return f.string(from: NSNumber(value: v)) ?? String(v)
    }

    /// The strings the app knows in `lang`, for the consistency check (`--l10n-check`).
    static func keys(_ lang: Lang) -> Set<String> { Set(load(lang).keys) }
}

/// Shorthand for the look-ups, so call sites read `Text(tr("save"))`.
func tr(_ key: String) -> String { L10n.t(key) }
func tr(_ key: String, _ args: CVarArg...) -> String { L10n.t(key, args) }
func trn(_ key: String, _ n: Int) -> String { L10n.plural(key, n) }

/// Marks a string literal as a translation key without looking it up yet (for tables whose
/// values are shown later through `tr(...)`). scripts/check-l10n.py reads these too.
func N_(_ key: String) -> String { key }
