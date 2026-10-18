import Foundation

/// The "cortex" log: a typewriter feed that narrates what the engine is actually doing.
final class Narrator {
    struct Context {
        var playing: Bool
        var sources: [SourceInfo]
        var telemetry: Telemetry
        var headActive: Bool
        var yawDegrees: Double
    }

    private struct Line {
        var text: String
        var shown: Double
        var color: UInt8
    }

    private var lines: [Line] = []
    private var nextAuto: Double = 0
    private var wasPlaying = false
    private var rng = LCG(seed: 0xC0_47E8)
    private let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f
    }()

    init() {
        say("boot asmr-core v0.1 ✓", color: Pal.dim)
        say("hrtf: brown–duda spherical head · ambisonic bus 3rd order (16ch) ✓", color: Pal.dim)
        say("standing by · press ▶ to enter the forest", color: Pal.text)
    }

    func say(_ text: String, color: UInt8 = Pal.text) {
        lines.append(Line(text: clock.string(from: Date()) + " " + text, shown: 0, color: color))
        if lines.count > 6 { lines.removeFirst(lines.count - 6) }
    }

    func update(t: Double, dt: Double, ctx: Context) {
        for i in lines.indices { lines[i].shown += dt * 60 }

        if ctx.playing != wasPlaying {
            wasPlaying = ctx.playing
            if ctx.playing {
                say("entering forest · river 4 m ahead · \(Int(ctx.telemetry.sampleRate / 1000)) kHz", color: Pal.bright)
            } else {
                say("field suspended", color: Pal.dim)
            }
            nextAuto = t + 2.5
        }
        guard ctx.playing, t >= nextAuto else { return }
        nextAuto = t + rng.range(3.5, 7.0)
        if let msg = generate(ctx) { say(msg.0, color: msg.1) }
    }

    private func generate(_ ctx: Context) -> (String, UInt8)? {
        let tel = ctx.telemetry
        var options: [(String, UInt8)] = []

        let birds = ctx.sources.filter { $0.kind == .bird }
        for (i, b) in birds.enumerated() where b.level > 0.01 {
            let az = atan2(Double(b.rel.y), Double(b.rel.x)) * 180 / .pi
            let side = az >= 0 ? "L" : "R"
            let dist = Double(simdLength(b.rel))
            let freq = b.freq > 0 ? "\(Int(b.freq)) Hz" : "song"
            options.append(("bird·\(i) \(freq) · \(side)\(Int(abs(az).rounded()))° · \(String(format: "%.1f", dist)) m", Pal.pink))
        }
        if tel.gust > 0.15 {
            options.append(("gust front W→E · u≈\(String(format: "%.1f", tel.gust * 6)) m/s", Pal.ramp(Pal.skyBase, 0.8)))
        }
        if tel.bubbleRate > 1 {
            options.append(("minnaert bank · \(Int(tel.bubbleRate)) bubbles/s · 0.4–5.4 kHz", Pal.cyan))
        }
        if ctx.headActive {
            options.append(("head yaw \(String(format: "%+.0f", ctx.yawDegrees))° · field counter-rotated", Pal.text))
        }
        options.append(("dsp \(String(format: "%.1f", tel.dspLoad * 100))% · \(ctx.sources.count) sources → 16ch SH → binaural", Pal.dim))
        options.append(([
            "acoustic field coherent",
            "listener immersion nominal",
            "spherical spreading 1/r · no walls detected",
            "canopy scattering estimated · forest is listening",
        ][rng.int(4)], Pal.dim))

        return options.isEmpty ? nil : options[rng.int(options.count)]
    }

    func render(into g: AsciiGrid, x: Int, y: Int, width: Int, rows: Int, t: Double) {
        let visible = lines.suffix(rows)
        for (row, line) in visible.enumerated() {
            let scalars = Array(line.text.unicodeScalars)
            let room = width - 3
            let n = min(scalars.count, Int(line.shown), room)
            g.put(x, y + row, "›", Pal.dim)
            var cx = x + 2
            for s in scalars.prefix(n) {
                g.put(cx, y + row, s, line.color)
                cx += 1
            }
            if n == room && scalars.count > room { g.put(cx - 1, y + row, "…", line.color) }
            let isLast = row == visible.count - 1
            if isLast && (Int(line.shown) < scalars.count || Int(t * 2) % 2 == 0) {
                g.put(cx, y + row, "▌", Pal.text)
            }
        }
    }
}

private func simdLength(_ v: SIMD3<Float>) -> Float {
    (v * v).sum().squareRoot()
}
