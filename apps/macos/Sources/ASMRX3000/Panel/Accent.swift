import AppKit
import Combine
import SwiftUI
import simd

/// The one colour in the app: it means "you" (the listener's point and gaze beam in the scene,
/// the head-tracking dot, the earbud and iris in the onboarding, the prompt while it records).
/// The user picks it from a short curated palette; violet is the default. Stored in
/// UserDefaults ("accent"), observable for SwiftUI, and readable from the render thread for
/// the scene's shader (`Accent.rgb`).
final class Accent: ObservableObject {
    static let shared = Accent()
    static let key = "accent"

    /// Muted, mid-luminance hues that read on the near-black panel and over the scene's grey fog
    /// without glowing. None is a pure primary; each sits near the same perceived brightness so
    /// switching does not change the panel's balance.
    enum Hue: String, CaseIterable, Identifiable {
        case violet, blue, teal, green, amber, red

        var id: String { rawValue }

        /// sRGB 0...1.
        var rgb: SIMD3<Float> {
            switch self {
            case .violet: return SIMD3(0.53, 0.33, 1.0)   // #8754FF, the app's listener violet
            case .blue: return SIMD3(0.35, 0.52, 0.96)    // #5985F5, a cold signal blue
            case .teal: return SIMD3(0.29, 0.70, 0.66)    // #4AB3A8, oxidised copper
            case .green: return SIMD3(0.58, 0.72, 0.36)   // #94B85C, lichen
            case .amber: return SIMD3(0.88, 0.63, 0.29)   // #E0A14A, a sodium street lamp
            case .red: return SIMD3(0.80, 0.27, 0.30)     // #CC454D, the series' red, lifted
            }
        }

        var color: Color { Color(red: Double(rgb.x), green: Double(rgb.y), blue: Double(rgb.z)) }

        /// What the picker says on hover and to VoiceOver.
        var name: String {
            switch self {
            case .violet: return N_("violet")
            case .blue: return N_("blue")
            case .teal: return N_("teal")
            case .green: return N_("green")
            case .amber: return N_("amber")
            case .red: return N_("red")
            }
        }
    }

    static let fallback = Hue.violet

    @Published var hue: Hue {
        didSet {
            guard hue != oldValue else { return }
            UserDefaults.standard.set(hue.rawValue, forKey: Self.key)
            Self.store(hue)
        }
    }

    private init() {
        let stored = UserDefaults.standard.string(forKey: Self.key).flatMap(Hue.init(rawValue:)) ?? Self.fallback
        hue = stored
        Self.store(stored)
    }

    // The current colour, for code that is not a SwiftUI view (CoreGraphics, Metal). Guarded
    // by a lock: the scene's renderer may read it off the main thread.
    private static let lock = NSLock()
    private static var currentHue = Accent.fallback

    private static func store(_ h: Hue) {
        lock.lock(); currentHue = h; lock.unlock()
    }

    static var current: Hue {
        lock.lock(); defer { lock.unlock() }
        return currentHue
    }

    static var color: Color { current.color }
    static var rgb: SIMD3<Float> { current.rgb }
}

/// A row of small round swatches; the chosen one has a ring. Click or arrow keys to pick.
struct AccentPicker: View {
    @ObservedObject var accent = Accent.shared
    @ObservedObject private var lang = Language.shared
    var size: CGFloat = 14

    var body: some View {
        HStack(spacing: Grid.x2) {
            ForEach(Accent.Hue.allCases) { h in
                Swatch(hue: h, chosen: accent.hue == h, size: size) { accent.hue = h }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(tr("Accent colour"))
    }

    private struct Swatch: View {
        let hue: Accent.Hue
        let chosen: Bool
        let size: CGFloat
        let action: () -> Void
        @ObservedObject private var lang = Language.shared
        @State private var hovering = false

        var body: some View {
            Button(action: action) {
                Circle()
                    .fill(hue.color)
                    .frame(width: size, height: size)
                    .padding(3)
                    .overlay(Circle().strokeBorder(chosen ? Ink.paper : (hovering ? Ink.graphite : .clear), lineWidth: 1))
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .focusable(false)
            .onHover { hovering = $0 }
            .help(tr(hue.name))
            .accessibilityLabel(tr(hue.name))
            .accessibilityAddTraits(chosen ? .isSelected : [])
        }
    }
}
