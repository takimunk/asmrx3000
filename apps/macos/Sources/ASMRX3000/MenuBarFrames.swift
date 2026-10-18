import AppKit
import ImageIO
import UniformTypeIdentifiers

/// `ASMRX3000 --menubar-frames dir`: renders the menu bar wordmark in each state on light and dark
/// bar backgrounds, at 2x, plus a contact sheet of the open-state crest. For visual checks.
enum MenuBarFrames {
    static func write(to dir: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let scale: CGFloat = 2
        let pad: CGFloat = 8
        let w = MenuBarIcon.size.width, h = MenuBarIcon.size.height
        func render(_ name: String, frames: [(NSImage?, CGFloat)], dark: Bool) {
            let cols = CGFloat(frames.count)
            let width = Int((w + pad * 2) * cols * scale), height = Int((h + pad) * scale)
            guard let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
            ctx.setFillColor(dark ? CGColor(srgbRed: 0.16, green: 0.16, blue: 0.17, alpha: 1)
                                  : CGColor(srgbRed: 0.93, green: 0.93, blue: 0.94, alpha: 1))
            ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
            ctx.scaleBy(x: scale, y: scale)
            for (i, (template, phase)) in frames.enumerated() {
                ctx.saveGState()
                ctx.translateBy(x: CGFloat(i) * (w + pad * 2) + pad, y: pad / 2)
                if let template {
                    // Draw a template the way the menu bar does: tinted to the bar's text colour.
                    var rect = CGRect(origin: .zero, size: MenuBarIcon.size)
                    if let cg = template.cgImage(forProposedRect: &rect, context: nil, hints: nil) {
                        ctx.clip(to: rect, mask: cg)
                        ctx.setFillColor(dark ? CGColor(gray: 1, alpha: 1) : CGColor(gray: 0, alpha: 0.85))
                        ctx.fill(rect)
                    }
                } else {
                    MenuBarIcon.draw(in: ctx, dark: dark, phase: phase)
                }
                ctx.restoreGState()
            }
            guard let image = ctx.makeImage(),
                  let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: dir).appendingPathComponent(name) as CFURL,
                                                             UTType.png.identifier as CFString, 1, nil) else { return }
            CGImageDestinationAddImage(dest, image, nil)
            CGImageDestinationFinalize(dest)
        }
        for dark in [false, true] {
            let tag = dark ? "dark" : "light"
            render("menubar-states-\(tag).png", frames: [(MenuBarIcon.stopped, 0), (MenuBarIcon.playing, 0), (nil, 0.3)], dark: dark)
            render("menubar-wave-\(tag).png", frames: (0..<10).map { (nil, CGFloat($0) / 10) }, dark: dark)
        }
        print("menubar frames → \(dir)")
    }
}
