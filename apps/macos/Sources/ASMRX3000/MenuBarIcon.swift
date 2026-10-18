import AppKit
import Combine
import QuartzCore

/// "ASMRX" hard-italic wordmark (geometry in MenuBarGlyphs.swift, generated from assets/icon/menubar.svg).
/// Idle: dimmed template image that follows the menu bar. Playing: solid letters with a violet band
/// sweeping left to right.
enum MenuBarIcon {
    static let size = NSSize(width: MenuBarGlyphs.size.width, height: MenuBarGlyphs.size.height)
    static let period: CFTimeInterval = 2.0

    static let idle: NSImage = {
        let image = NSImage(size: size, flipped: false) { _ in
            guard let ctx = NSGraphicsContext.current?.cgContext else { return false }
            ctx.addPath(MenuBarGlyphs.path)
            ctx.setFillColor(NSColor.black.withAlphaComponent(0.55).cgColor)
            ctx.fillPath(using: .evenOdd)
            return true
        }
        image.isTemplate = true
        return image
    }()

    /// One frame of the active animation. Not a template: colours are resolved at draw time against
    /// the appearance the image is drawn in (the status item's), so it reads on light and dark bars.
    static func active(phase: CGFloat) -> NSImage {
        let image = NSImage(size: size, flipped: false) { _ in
            guard let ctx = NSGraphicsContext.current?.cgContext else { return false }
            let appearance = NSAppearance.currentDrawing()
            let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            draw(in: ctx, dark: dark, phase: phase)
            return true
        }
        image.isTemplate = false
        return image
    }

    /// `phase` in 0..<1: position of the band across the word.
    static func draw(in ctx: CGContext, dark: Bool, phase: CGFloat) {
        let path = MenuBarGlyphs.path
        let base = dark ? CGColor(gray: 1, alpha: 0.88) : CGColor(gray: 0, alpha: 0.85)
        ctx.saveGState()
        ctx.addPath(path)
        ctx.setFillColor(base)
        ctx.fillPath(using: .evenOdd)

        ctx.addPath(path)
        ctx.clip(using: .evenOdd)
        // Shear the gradient space so the band's edges run parallel to the italic stems.
        ctx.concatenate(CGAffineTransform(a: 1, b: 0, c: MenuBarGlyphs.slant, d: 1, tx: 0, ty: 0))
        let band: CGFloat = max(12, size.width * 0.3)
        let w = size.width
        let travel = w + MenuBarGlyphs.slant * size.height + band * 2
        let center = -band + phase * travel
        let gradient = dark ? darkBand : lightBand
        ctx.drawLinearGradient(gradient,
                               start: CGPoint(x: center - band / 2, y: 0),
                               end: CGPoint(x: center + band / 2, y: 0),
                               options: [])
        ctx.restoreGState()
    }

    private static func bandGradient(edge: CGColor, core: CGColor) -> CGGradient {
        let clear = edge.copy(alpha: 0)!
        return CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
                          colors: [clear, edge, core, edge, clear] as CFArray,
                          locations: [0, 0.3, 0.5, 0.7, 1])!
    }

    private static let darkBand = bandGradient(
        edge: CGColor(srgbRed: 0.62, green: 0.38, blue: 1.0, alpha: 1),
        core: CGColor(srgbRed: 0.80, green: 0.66, blue: 1.0, alpha: 1))
    private static let lightBand = bandGradient(
        edge: CGColor(srgbRed: 0.50, green: 0.16, blue: 0.98, alpha: 1),
        core: CGColor(srgbRed: 0.62, green: 0.30, blue: 1.0, alpha: 1))
}

/// Publishes the current menu bar image; runs a ~30 fps timer only while active.
final class MenuBarIconAnimator: ObservableObject {
    @Published private(set) var image: NSImage = MenuBarIcon.idle
    private var timer: Timer?

    var active = false {
        didSet {
            guard active != oldValue else { return }
            active ? start() : stop()
        }
    }

    private func start() {
        tick()
        let t = Timer(timeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in self?.tick() }
        t.tolerance = 0.005
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    private func stop() {
        timer?.invalidate()
        timer = nil
        image = MenuBarIcon.idle
    }

    private func tick() {
        let now = CACurrentMediaTime()
        let phase = CGFloat(now.truncatingRemainder(dividingBy: MenuBarIcon.period) / MenuBarIcon.period)
        image = MenuBarIcon.active(phase: phase)
    }

    deinit { timer?.invalidate() }
}
