import SwiftUI

/// The interface language, as a named row of two text choices; the picked one is outlined.
/// Switching applies at once, everywhere (Language.shared); the onboarding's first step sets
/// it the first time.
struct LanguageRow: View {
    var nameWidth: CGFloat = 112
    @ObservedObject private var lang = Language.shared

    var body: some View {
        HStack(spacing: Grid.x2) {
            Text(tr("Language"))
                .foregroundStyle(Ink.graphite)
                .frame(width: nameWidth, alignment: .leading)
            ForEach(Lang.allCases) { l in
                TextButton(title: l.native, strong: lang.current == l) { Language.shared.select(l) }
            }
            Spacer(minLength: 0)
        }
        .font(Ink.text(Ink.Size.body))
        .frame(height: 28)
    }
}
