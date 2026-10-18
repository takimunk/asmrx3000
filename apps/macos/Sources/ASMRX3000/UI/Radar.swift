import Foundation

/// Top-down view of the sound field around the listener's head, in head coordinates:
/// up is where your nose points. Turn your head with AirPods on and the world rotates.
final class Radar {
    let w: Int
    let h: Int
    private var canvas: BrailleCanvas
    private var sweep = 0.0

    init(w: Int, h: Int) {
        self.w = w
        self.h = h
        canvas = BrailleCanvas(w: w, h: h)
    }

    func render(into g: AsciiGrid, x ox: Int, y oy: Int, sources: [SourceInfo],
                yaw: Double, t: Double, dt: Double, playing: Bool) {
        canvas.clear()
        let cx = Double(canvas.pixelWidth) / 2 - 0.5
        let cy = Double(canvas.pixelHeight) / 2 - 0.5
        let r = min(cx, cy) - 0.5

        func dot(_ px: Double, _ py: Double, _ c: UInt8, _ p: UInt8) {
            canvas.set(Int(px.rounded()), Int(py.rounded()), c, prio: p)
        }

        let steps = 96
        for i in 0..<steps {
            let a = Double(i) / Double(steps) * 2 * .pi
            dot(cx + r * cos(a), cy + r * sin(a), Pal.faint, 1)
            if i % 2 == 0 { dot(cx + r * 0.5 * cos(a), cy + r * 0.5 * sin(a), Pal.faint, 1) }
        }
        for d in stride(from: -r, through: r, by: 2) {
            dot(cx + d, cy, Pal.faint, 1)
            dot(cx, cy + d, Pal.faint, 1)
        }

        if playing { sweep += dt * 2.2 }
        for k in 0..<6 {
            let a = sweep - Double(k) * 0.09
            let color = k == 0 ? Pal.text : Pal.dim
            for rr in stride(from: 2.0, through: r, by: 1.0) {
                dot(cx + rr * sin(a), cy - rr * cos(a), color, UInt8(4 - min(k, 3)))
            }
        }

        let logMax = log(1 + 22.0)
        for s in sources {
            let rx = Double(s.rel.x), ry = Double(s.rel.y), rz = Double(s.rel.z)
            let d = sqrt(rx * rx + ry * ry + rz * rz)
            let az = atan2(ry, rx)
            let rr = r * log(1 + d) / logMax
            let px = cx - rr * sin(az)
            let py = cy - rr * cos(az)
            switch s.kind {
            case .water:
                let c = Pal.ramp(Pal.blueBase, 0.75)
                dot(px, py, c, 6)
                dot(px + 1, py, c, 6)
            case .wind:
                dot(px, py, Pal.ramp(Pal.skyBase, 0.55), 5)
            case .bird:
                let singing = playing && s.level > 0.01
                let blink = !singing || Int(t * 6) % 2 == 0
                let c = singing ? Pal.pink : Pal.amber
                if blink {
                    dot(px, py, c, 8)
                    dot(px + 1, py, c, 8)
                    dot(px, py + 1, c, 8)
                    dot(px + 1, py + 1, c, 8)
                }
            }
        }

        // The listener and their nose.
        for (dx, dy) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
            dot(cx + dx, cy + dy, Pal.bright, 9)
        }
        dot(cx - 0.5, cy - 2.5, Pal.bright, 9)
        dot(cx + 0.5, cy - 2.5, Pal.bright, 9)

        canvas.blit(into: g, x: ox, y: oy)

        // World "north" (scene forward) marker rides the outer ring as the head turns.
        let a = -yaw
        let nx = cx - r * sin(a), ny = cy - r * cos(a)
        g.put(ox + Int(nx / 2), oy + Int(ny / 4), "N", Pal.amber)
    }
}
