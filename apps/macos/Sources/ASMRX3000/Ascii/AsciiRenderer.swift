import AppKit
import CoreText
import SwiftUI

/// Glyph lookup with font fallback. Fallback glyphs (Braille, symbols) that are wider than a
/// cell are scaled down so the grid never drifts.
final class GlyphAtlas {
    static let shared = GlyphAtlas(size: 11)

    let font: CTFont
    let cellW: CGFloat
    let cellH: CGFloat
    let ascent: CGFloat
    private var cache: [UInt32: (font: CTFont, glyph: CGGlyph, dx: CGFloat)] = [:]

    init(size: CGFloat) {
        font = NSFont.monospacedSystemFont(ofSize: size, weight: .regular) as CTFont
        var ch: UniChar = 0x4D // "M"
        var g = CGGlyph()
        CTFontGetGlyphsForCharacters(font, &ch, &g, 1)
        var adv = CGSize.zero
        CTFontGetAdvancesForGlyphs(font, .horizontal, &g, &adv, 1)
        cellW = adv.width
        ascent = CTFontGetAscent(font)
        cellH = ceil(ascent + CTFontGetDescent(font))
    }

    func glyph(_ scalar: UInt32) -> (font: CTFont, glyph: CGGlyph, dx: CGFloat)? {
        if let hit = cache[scalar] { return hit }
        guard let us = Unicode.Scalar(scalar) else { return nil }
        let str = String(Character(us))
        var units = Array(str.utf16)
        var glyphs = [CGGlyph](repeating: 0, count: units.count)
        var f = font
        if !CTFontGetGlyphsForCharacters(f, &units, &glyphs, units.count) {
            f = CTFontCreateForString(font, str as CFString, CFRange(location: 0, length: units.count))
            CTFontGetGlyphsForCharacters(f, &units, &glyphs, units.count)
        }
        var adv = CGSize.zero
        CTFontGetAdvancesForGlyphs(f, .horizontal, &glyphs, &adv, 1)
        if adv.width > cellW * 1.05 {
            f = CTFontCreateCopyWithAttributes(f, CTFontGetSize(f) * cellW / adv.width, nil, nil)
            CTFontGetAdvancesForGlyphs(f, .horizontal, &glyphs, &adv, 1)
        }
        let entry = (font: f, glyph: glyphs[0], dx: (cellW - adv.width) / 2)
        cache[scalar] = entry
        return entry
    }
}

enum AsciiRenderer {
    static func draw(_ grid: AsciiGrid, in cg: CGContext) {
        let atlas = GlyphAtlas.shared
        let cw = atlas.cellW, ch = atlas.cellH
        let colors = Pal.colors

        for y in 0..<grid.rows {
            for x in 0..<grid.cols {
                let b = grid.bg[y * grid.cols + x]
                if b != 0 {
                    cg.setFillColor(colors[Int(b)])
                    cg.fill(CGRect(x: CGFloat(x) * cw, y: CGFloat(y) * ch, width: cw + 0.5, height: ch))
                }
            }
        }

        // SwiftUI's context is y-down; flip glyphs so they stand upright. The text matrix also
        // applies to glyph positions, so baselines are passed negated.
        cg.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        var lastColor: UInt8 = 255
        for y in 0..<grid.rows {
            let baseline = CGFloat(y) * ch + atlas.ascent
            for x in 0..<grid.cols {
                let i = y * grid.cols + x
                let g = grid.glyphs[i]
                if g == 32 || g == 0 { continue }
                guard let entry = atlas.glyph(g) else { continue }
                let c = grid.fg[i]
                if c != lastColor {
                    cg.setFillColor(colors[Int(c == 0 ? Pal.text : c)])
                    lastColor = c
                }
                var glyph = entry.glyph
                var pos = CGPoint(x: CGFloat(x) * cw + entry.dx, y: -baseline)
                CTFontDrawGlyphs(entry.font, &glyph, &pos, 1, cg)
            }
        }
    }
}

/// The whole popover: one animated character grid with CRT glow and scanlines.
struct ConsoleView: View {
    let console: Console
    @State private var visible = false
    private let pad: CGFloat = 10

    var body: some View {
        let atlas = GlyphAtlas.shared
        let width = CGFloat(console.cols) * atlas.cellW
        let height = CGFloat(console.rows) * atlas.cellH

        ZStack {
            Color(cgColor: Pal.background)
            TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: !visible)) { timeline in
                Canvas(rendersAsynchronously: false) { ctx, _ in
                    let grid = console.frame(time: timeline.date.timeIntervalSinceReferenceDate)
                    ctx.withCGContext { cg in AsciiRenderer.draw(grid, in: cg) }
                }
            }
            .frame(width: width, height: height)
            .compositingGroup()
            .shadow(color: Color(red: 0.4, green: 1.0, blue: 0.6).opacity(0.35), radius: 3)
            Scanlines().allowsHitTesting(false)
            RadialGradient(colors: [.clear, .black.opacity(0.35)], center: .center,
                           startRadius: width * 0.35, endRadius: width * 0.85)
                .allowsHitTesting(false)
        }
        .frame(width: width + pad * 2, height: height + pad * 2)
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 0, coordinateSpace: .local)
                .onChanged { v in
                    let c = cell(v.location)
                    console.pointer(x: c.x, y: c.y, phase: .drag)
                }
                .onEnded { v in
                    let c = cell(v.location)
                    console.pointer(x: c.x, y: c.y, phase: .up)
                }
        )
        .onContinuousHover { phase in
            switch phase {
            case .active(let p):
                let c = cell(p)
                console.hover(x: c.x, y: c.y)
            case .ended:
                console.hover(x: -1, y: -1)
            }
        }
        .onAppear { visible = true }
        .onDisappear { visible = false }
    }

    private func cell(_ p: CGPoint) -> (x: Double, y: Double) {
        let atlas = GlyphAtlas.shared
        return (Double((p.x - pad) / atlas.cellW), Double((p.y - pad) / atlas.cellH))
    }
}

private struct Scanlines: View {
    var body: some View {
        Canvas { ctx, size in
            var y: CGFloat = 0
            while y < size.height {
                ctx.fill(Path(CGRect(x: 0, y: y, width: size.width, height: 1)), with: .color(.black.opacity(0.16)))
                y += 3
            }
        }
    }
}
