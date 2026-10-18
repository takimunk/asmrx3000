import AppKit

/// A pine tree built from waveform bars: forest + sound in 18 pt.
enum MenuBarIcon {
    private static let idle = make(alpha: 0.55)
    private static let live = make(alpha: 1.0)

    static func image(active: Bool) -> NSImage {
        active ? live : idle
    }

    private static func make(alpha: CGFloat) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            NSColor.black.withAlphaComponent(alpha).setFill()
            let widths: [CGFloat] = [14, 11, 8, 5, 2.4]
            let barHeight: CGFloat = 1.8
            let gap: CGFloat = 1.3
            var y: CGFloat = 3.6
            for w in widths {
                let rect = NSRect(x: (18 - w) / 2, y: y, width: w, height: barHeight)
                NSBezierPath(roundedRect: rect, xRadius: 0.9, yRadius: 0.9).fill()
                y += barHeight + gap
            }
            NSBezierPath(roundedRect: NSRect(x: 8, y: 0.8, width: 2, height: 2.4), xRadius: 0.5, yRadius: 0.5).fill()
            return true
        }
        image.isTemplate = true
        return image
    }
}
