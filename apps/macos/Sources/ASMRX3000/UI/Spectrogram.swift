/// Scrolling spectrogram drawn in Braille: 2x4 dots per cell, ordered dithering for
/// intensity, cell colour from an inferno heat map. Time runs left to right, low
/// frequencies at the bottom.
final class Spectrogram {
    let w: Int
    let h: Int
    let bands: Int
    private var columns: [[Float]]
    private var head = 0
    private var accum = 0.0
    private var canvas: BrailleCanvas
    private static let columnsPerSecond = 20.0

    // 2x4 ordered-dither thresholds, one per dot position.
    private static let bayer: [[Float]] = [[0, 4], [6, 2], [1, 5], [7, 3]].map { row in
        row.map { (Float($0) + 0.5) / 8 * 0.8 + 0.06 }
    }

    init(w: Int, h: Int) {
        self.w = w
        self.h = h
        bands = h * 4
        columns = Array(repeating: Array(repeating: 0, count: h * 4), count: w * 2)
        canvas = BrailleCanvas(w: w, h: h)
    }

    func push(_ spectrum: [Float], dt: Double) {
        accum += dt
        while accum >= 1 / Self.columnsPerSecond {
            accum -= 1 / Self.columnsPerSecond
            columns[head] = spectrum
            head = (head + 1) % columns.count
        }
    }

    func render(into g: AsciiGrid, x: Int, y: Int) {
        canvas.clear()
        let n = columns.count
        for c in 0..<n {
            let col = columns[(head + c) % n]
            for b in 0..<min(bands, col.count) {
                let lifted = max(0, (col[b] - 0.15) / 0.85) // drop the noise floor
                let v = lifted * lifted.squareRoot() // contrast curve
                let py = bands - 1 - b
                if v > Self.bayer[py & 3][c & 1] {
                    canvas.set(c, py, Pal.heat(v), prio: UInt8(min(255, v * 255)))
                }
            }
        }
        canvas.blit(into: g, x: x, y: y)
    }
}
