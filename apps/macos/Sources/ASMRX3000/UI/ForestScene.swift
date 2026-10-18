import Foundation
import simd

/// Side view of the forest by the river. Everything that moves is driven by engine telemetry:
/// trees and grass bend with the same travelling gust front the wind sources hear, birds sit
/// where the engine puts them and emit notes while they actually sing, and water sparkles
/// with the bubble rate.
final class ForestScene {
    let w: Int
    let h: Int
    private let groundRow: Int

    private struct Tree {
        let x: Int
        let height: Int
        let pine: Bool
        let width: Int
        let phase: Double
    }

    private struct Particle {
        var x: Double
        var y: Double
        var vx: Double
        var vy: Double
        var life: Double
        let maxLife: Double
        let glyph: Unicode.Scalar
        let color: UInt8
        let fadeColor: UInt8
        let wobble: Bool
    }

    private let trees: [Tree]
    private let ridge: [Int]
    private var particles: [Particle] = []
    private var flow = 0.0
    private var clouds = 0.0
    private var noteTimers = [Double](repeating: 0, count: 8)
    private var lastBirdPos = [SIMD3<Float>](repeating: .zero, count: 8)
    private var leafBudget = 0.0
    private var sparkleBudget = 0.0
    private var rng = LCG(seed: 0xF0_2E57)

    private static let leafGlyphs: [Unicode.Scalar] = ["♣", "♠", "&", "%", "*", "@"]
    private static let noteGlyphs: [Unicode.Scalar] = ["♪", "♫", "♩", "♬"]
    private static let fallingLeaves: [Unicode.Scalar] = ["❦", "❧", "‚", "˛"]
    private static let sparkles: [Unicode.Scalar] = ["·", "✧", "∙", "*"]

    init(w: Int, h: Int) {
        self.w = w
        self.h = h
        groundRow = h - 4
        let g = h - 4
        ridge = (0..<w).map { x in
            let fx = Double(x)
            let v = 1.6 * sin(fx * 0.31) + 1.1 * sin(fx * 0.83 + 1.3) + 1.0
            return max(3, min(g - 2, g - 4 - Int(v.rounded())))
        }
        trees = [
            Tree(x: 5, height: 10, pine: true, width: 4, phase: 0.0),
            Tree(x: 12, height: 8, pine: false, width: 4, phase: 1.3),
            Tree(x: 20, height: 11, pine: true, width: 4, phase: 2.1),
            Tree(x: 29, height: 7, pine: false, width: 5, phase: 0.7),
            Tree(x: 37, height: 9, pine: true, width: 4, phase: 2.9),
            Tree(x: 45, height: 9, pine: false, width: 5, phase: 1.9),
            Tree(x: 52, height: 11, pine: true, width: 4, phase: 0.4),
        ].filter { $0.x < w }
    }

    func render(into g: AsciiGrid, x ox: Int, y oy: Int, t: Double, dt: Double,
                wind: [Float], windParam: Float, gust: Float, water: Float,
                sources: [SourceInfo], playing: Bool) {
        func put(_ x: Int, _ y: Int, _ s: Unicode.Scalar, _ c: UInt8) {
            if x >= 0, x < w, y >= 0, y < h { g.put(ox + x, oy + y, s, c) }
        }
        func local(_ col: Int) -> Double {
            wind.isEmpty ? 0 : Double(wind[max(0, min(wind.count - 1, col * wind.count / max(1, w)))])
        }

        clouds += dt * (0.4 + Double(windParam) * 2.0)
        flow += dt * (0.8 + Double(water) * 2.5)

        // Sun first, clouds drift in front of it.
        let sunX = w - 7
        put(sunX, 1, "☼", Pal.sun)
        if Int(t * 2) % 2 == 0 {
            put(sunX - 2, 1, "·", Pal.amber)
            put(sunX + 2, 1, "·", Pal.amber)
            put(sunX, 0, "·", Pal.amber)
        } else {
            put(sunX - 1, 0, "˙", Pal.amber)
            put(sunX + 1, 2, "˙", Pal.amber)
        }

        for row in 0..<4 {
            for x in 0..<w {
                let n = fbm(Double(x) * 0.11 - clouds * 0.35, Double(row) * 0.55 + 3.0)
                let v = n - (0.50 + 0.07 * Double(row))
                if v > 0.16 {
                    put(x, row, "▓", Pal.ramp(Pal.skyBase, 0.8))
                } else if v > 0.08 {
                    put(x, row, "▒", Pal.ramp(Pal.skyBase, 0.6))
                } else if v > 0.0 {
                    put(x, row, "░", Pal.ramp(Pal.skyBase, 0.4))
                }
            }
        }

        // Distant tree line.
        for x in 0..<w {
            let top = ridge[x]
            put(x, top, "▴", Pal.ramp(Pal.greenBase, 0.2))
            for row in (top + 1)..<groundRow {
                let n = vnoise(Double(x) * 0.8 + t * 0.15, Double(row) * 1.1)
                put(x, row, n > 0.72 ? "▒" : "░", Pal.ramp(Pal.greenBase, Float(0.05 + 0.12 * n)))
            }
        }

        for tree in trees {
            drawTree(tree, put: put, gust: local(tree.x), t: t)
        }

        // Grass leaning with the local gust.
        for x in 0..<w {
            let gx = local(x)
            let n = vnoise(Double(x) * 0.7 + t * 0.8, 9.1)
            let lean = gx * 1.3 + (n - 0.5) * 0.5 + 0.3 * gx * sin(t * 3.1 + Double(x) * 0.45)
            let glyph: Unicode.Scalar
            if lean < 0.22 { glyph = x % 3 == 0 ? "ı" : "|" }
            else if lean < 0.65 { glyph = "/" }
            else if lean < 1.15 { glyph = "╱" }
            else { glyph = "_" }
            put(x, groundRow, glyph, Pal.ramp(Pal.greenBase, Float(0.3 + 0.5 * n)))
            if x % 13 == 6 && lean < 0.65 { put(x, groundRow, "✿", x % 2 == 0 ? Pal.pink : Pal.amber) }
        }

        // River flowing left to right.
        let waterRows = h - groundRow - 1
        for d in 0..<waterRows {
            let row = groundRow + 1 + d
            for x in 0..<w {
                let fx = Double(x)
                let n = vnoise(fx * 0.35 - flow * (1.0 + 0.35 * Double(d)), Double(d) * 1.9 + t * 0.25)
                let wave = 0.5 + 0.5 * sin(fx * 0.55 - flow * 2.6 + Double(d) * 2.1)
                let m = 0.6 * n + 0.4 * wave
                let glyph: Unicode.Scalar = m < 0.3 ? "˜" : m < 0.52 ? "~" : m < 0.72 ? "≈" : "≋"
                let shade = max(0, min(1, 0.2 + m * 0.6 - Double(d) * 0.12))
                put(x, row, glyph, Pal.ramp(Pal.blueBase, Float(shade)))
            }
        }

        // Sparkles on the water.
        sparkleBudget += dt * (playing ? 2 + 18 * Double(water) : 1)
        while sparkleBudget >= 1 {
            sparkleBudget -= 1
            let life = rng.range(0.2, 0.5)
            particles.append(Particle(
                x: rng.range(0, Double(w)), y: Double(groundRow + 1 + rng.int(max(1, waterRows))),
                vx: 0.8 + Double(water) * 2.5, vy: 0, life: life, maxLife: life,
                glyph: Self.sparkles[rng.int(Self.sparkles.count)], color: Pal.bright, fadeColor: Pal.cyan, wobble: false))
        }

        // Leaves torn off by strong gusts.
        leafBudget += dt * max(0, Double(gust) - 0.35) * 10
        while leafBudget >= 1, !trees.isEmpty {
            leafBudget -= 1
            let tree = trees[rng.int(trees.count)]
            let gx = local(tree.x)
            let life = 6.0
            particles.append(Particle(
                x: Double(tree.x) + rng.range(-2, 2), y: Double(groundRow - tree.height + 1 + rng.int(4)),
                vx: 3 + 9 * gx, vy: rng.range(0.6, 1.6), life: life, maxLife: life,
                glyph: Self.fallingLeaves[rng.int(Self.fallingLeaves.count)],
                color: Pal.amber, fadeColor: Pal.trunk, wobble: true))
        }

        // Birds where the engine says they are; notes while they sing.
        var birdIndex = 0
        for s in sources where s.kind == .bird {
            defer { birdIndex += 1 }
            guard birdIndex < noteTimers.count else { break }
            let col = Int(((20 - Double(s.world.y)) / 40 * Double(w)).rounded())
            let row = max(1, min(groundRow - 1, groundRow - 1 - Int((Double(s.world.z) * 1.35).rounded())))
            let moved = simd_length_squared(s.world - lastBirdPos[birdIndex]) > 1e-6
            lastBirdPos[birdIndex] = s.world
            let singing = playing && s.level > 0.01
            let glyph: Unicode.Scalar = moved ? (Int(t * 8) % 2 == 0 ? "v" : "ʌ") : "♦"
            put(col, row, glyph, singing ? Pal.pink : Pal.amber)
            if singing {
                noteTimers[birdIndex] -= dt
                if noteTimers[birdIndex] <= 0 {
                    noteTimers[birdIndex] = rng.range(0.18, 0.33)
                    particles.append(Particle(
                        x: Double(col + 1), y: Double(row - 1), vx: 1.5 + Double(gust) * 3, vy: -1.6,
                        life: 1.6, maxLife: 1.6, glyph: Self.noteGlyphs[rng.int(Self.noteGlyphs.count)],
                        color: Pal.pink, fadeColor: Pal.ramp(Pal.titleBase, 0.7), wobble: true))
                }
            }
        }

        // Particles.
        var alive: [Particle] = []
        alive.reserveCapacity(particles.count)
        for var p in particles {
            p.life -= dt
            p.x += p.vx * dt
            p.y += p.vy * dt + (p.wobble ? sin(t * 5 + p.x) * 0.02 : 0)
            let groundHit = p.glyph != "·" && p.vy > 0 && p.y >= Double(groundRow + 1)
            guard p.life > 0, p.x < Double(w), p.y >= 0, !groundHit else { continue }
            put(Int(p.x), Int(p.y), p.glyph, p.life < p.maxLife * 0.35 ? p.fadeColor : p.color)
            alive.append(p)
        }
        particles = alive
    }

    private func drawTree(_ tree: Tree, put: (Int, Int, Unicode.Scalar, UInt8) -> Void, gust gx: Double, t: Double) {
        let trunkH = tree.pine ? 1 : 2
        let trunkTop = groundRow - trunkH
        let top = groundRow - tree.height
        let span = Double(max(1, trunkTop - top))

        for row in top..<trunkTop {
            let k = row - top
            let f = Double(trunkTop - row) / span // 1 at the crown
            let stiffness = tree.pine ? 0.7 : 1.0
            let sway = stiffness * (gx * 1.5 * pow(f, 1.4) + 0.45 * gx * sin(t * 2.4 + tree.phase + f * 1.2) * f)
            let dx = Int(sway.rounded())

            let hw: Int
            if tree.pine {
                hw = min(tree.width, max(0, Int(Double(k) * 0.55) - (k % 3 == 2 ? 1 : 0)))
            } else {
                let ry = span / 2
                let cy = Double(top) + ry - 0.5
                let e = (Double(row) - cy) / ry
                hw = Int((Double(tree.width) * sqrt(max(0, 1 - e * e))).rounded())
            }

            for i in -hw...hw {
                let cx = tree.x + i + dx
                let n = vnoise(Double(cx) * 0.9 + t * (0.3 + gx * 2.5), Double(row) * 1.3)
                let light = 0.5 - Double(i) / Double(max(hw, 1)) * 0.25 + f * 0.15
                let shade = min(1, max(0, 0.35 * n + 0.65 * light))
                let color = Pal.ramp(Pal.greenBase, Float(0.15 + 0.7 * shade))
                if tree.pine {
                    let glyph: Unicode.Scalar
                    if k == 0 || hw == 0 { glyph = "▲" }
                    else if i == -hw { glyph = "◢" }
                    else if i == hw { glyph = "◣" }
                    else { glyph = n > 0.62 ? "▓" : (n > 0.35 ? "▒" : "░") }
                    put(cx, row, glyph, color)
                } else {
                    if abs(i) == hw && n < 0.3 { continue }
                    let idx = min(Self.leafGlyphs.count - 1, Int(n * Double(Self.leafGlyphs.count)))
                    put(cx, row, Self.leafGlyphs[idx], color)
                }
            }
        }

        for row in trunkTop..<groundRow {
            put(tree.x, row, "┃", Pal.trunk)
        }
        if !tree.pine {
            put(tree.x - 1, trunkTop, "╲", Pal.trunk)
            put(tree.x + 1, trunkTop, "╱", Pal.trunk)
        }
    }
}

