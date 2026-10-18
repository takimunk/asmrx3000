import AppKit
import ImageIO
import UniformTypeIdentifiers

/// `ASMRX3000 --snapshot out.png`: runs the engine and console for a few seconds without an
/// audio device and writes one console frame as a PNG. For visual checks and CI.
enum Snapshot {
    static func run(path: String, seconds: Double = 6) {
        let model = AppModel()
        model.audio.orbit = 0.4
        let console = model.console
        let fps = 30.0
        var grid: AsciiGrid?
        for i in 0..<Int(seconds * fps) {
            model.audio.simulate(seconds: 1 / fps)
            grid = console.frame(time: 1000 + Double(i) / fps)
        }
        guard let grid else { return }

        let atlas = GlyphAtlas.shared
        let scale: CGFloat = 2
        let pad: CGFloat = 10
        let w = CGFloat(grid.cols) * atlas.cellW + pad * 2
        let h = CGFloat(grid.rows) * atlas.cellH + pad * 2
        guard let ctx = CGContext(data: nil, width: Int(w * scale), height: Int(h * scale), bitsPerComponent: 8,
                                  bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return }
        ctx.setFillColor(Pal.background)
        ctx.fill(CGRect(x: 0, y: 0, width: w * scale, height: h * scale))
        // Match SwiftUI's y-down coordinate space.
        ctx.translateBy(x: 0, y: h * scale)
        ctx.scaleBy(x: scale, y: -scale)
        ctx.translateBy(x: pad, y: pad)
        AsciiRenderer.draw(grid, in: ctx)

        guard let image = ctx.makeImage(),
              let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: path) as CFURL,
                                                         UTType.png.identifier as CFString, 1, nil)
        else { return }
        CGImageDestinationAddImage(dest, image, nil)
        CGImageDestinationFinalize(dest)
        print("snapshot → \(path)")
    }
}
