import AppKit

/// Composes the whole popover as one character grid and handles pointer input on it.
///
///   row  0-1   title / status
///   row  2-19  scene panel (56x16 forest by the river)
///   row 20-29  spectrogram (36x8 braille) | sound-field radar (18x8 braille)
///   row 30     transport buttons
///   row 32-36  sliders
///   row 37     separator
///   row 38-40  cortex log
final class Console {
    enum PointerPhase { case drag, up }

    enum SliderID: CaseIterable {
        case volume, wind, birds, water, orbit

        var label: String {
            switch self {
            case .volume: return "VOLUME"
            case .wind: return "WIND"
            case .birds: return "BIRDS"
            case .water: return "WATER"
            case .orbit: return "ORBIT"
            }
        }

        var ramp: UInt8 {
            switch self {
            case .volume: return Pal.greenBase
            case .wind: return Pal.skyBase
            case .birds: return Pal.heatBase
            case .water: return Pal.blueBase
            case .orbit: return Pal.titleBase
            }
        }
    }

    enum Control: Equatable {
        case play, recenter, headTrack, quit
        case slider(SliderID)
    }

    private struct Region {
        let x: Int, y: Int, w: Int, h: Int
        let control: Control
    }

    let cols = 58
    let rows = 41

    private unowned let model: AppModel
    private let grid: AsciiGrid
    private let forest: ForestScene
    private let spectro: Spectrogram
    private let radar: Radar
    private let narrator = Narrator()

    private var regions: [Region] = []
    private var hovered: Control?
    private var pressed: Control?
    private var lastTime: Double = 0

    private var spectrum = [Float](repeating: 0, count: 32)
    private var windField = [Float](repeating: 0, count: 56)
    private var sources: [SourceInfo] = []

    private static let sliderX = 10
    private static let sliderW = 36
    private static let partials: [Unicode.Scalar] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉"]
    private let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f
    }()

    init(model: AppModel) {
        self.model = model
        grid = AsciiGrid(cols: cols, rows: rows)
        forest = ForestScene(w: cols - 2, h: 16)
        spectro = Spectrogram(w: 36, h: 8)
        radar = Radar(w: 18, h: 8)
        spectrum = [Float](repeating: 0, count: spectro.bands)
    }

    // MARK: Frame

    func frame(time t: Double) -> AsciiGrid {
        let dt = lastTime == 0 ? 1.0 / 30.0 : min(0.1, max(0, t - lastTime))
        lastTime = t
        let audio = model.audio
        let head = model.head
        let tel = audio.readTelemetry(spectrum: &spectrum, windField: &windField, sources: &sources)

        grid.clear()
        regions.removeAll(keepingCapacity: true)

        drawHeader(t: t, playing: audio.isPlaying)

        grid.box(0, 2, cols, 18, color: Pal.dim, title: " SCENE ▸ FOREST · RIVER ",
                 right: " ☀ " + clock.string(from: Date()) + " ")
        forest.render(into: grid, x: 1, y: 3, t: t, dt: dt, wind: windField, windParam: audio.wind,
                      gust: tel.gust, water: audio.water, sources: sources, playing: audio.isPlaying)
        drawStatus(y: 19, tel: tel, t: t)

        grid.box(0, 20, 38, 10, color: Pal.dim, title: " SPECTROGRAM ", right: " 40Hz–16k ")
        spectro.push(spectrum, dt: dt)
        spectro.render(into: grid, x: 1, y: 21)

        grid.box(38, 20, 20, 10, color: Pal.dim, title: " FIELD ", right: head.active ? " ◉ " : " ○ ",
                 rightColor: head.active ? Pal.text : Pal.faint)
        radar.render(into: grid, x: 39, y: 21, sources: sources, yaw: head.yaw, t: t, dt: dt, playing: audio.isPlaying)

        drawControls()

        let sepY = 37
        for x in 0..<cols { grid.put(x, sepY, "─", Pal.faint) }
        grid.text(2, sepY, " cortex ", Pal.dim)
        narrator.update(t: t, dt: dt, ctx: .init(playing: audio.isPlaying, sources: sources, telemetry: tel,
                                                 headActive: head.active, yawDegrees: head.yaw * 180 / .pi))
        narrator.render(into: grid, x: 1, y: 38, width: cols - 1, rows: 3, t: t)
        return grid
    }

    private func drawHeader(t: Double, playing: Bool) {
        let title = Array("▞▚ ASMR X3000 ▞▚".unicodeScalars)
        let glitch = Int(t * 12) % 53 == 0 ? Int(t * 7) % title.count : -1
        for (i, s) in title.enumerated() {
            var phase = (Double(i) * 0.06 - t * 0.25).truncatingRemainder(dividingBy: 1)
            if phase < 0 { phase += 1 }
            grid.put(1 + i, 0, i == glitch ? "░" : s, Pal.ramp(Pal.titleBase, Float(phase)))
        }
        if playing {
            let on = Int(t * 2) % 2 == 0
            grid.text(cols - 9, 0, on ? "● LIVE" : "○ LIVE", on ? Pal.red : Pal.dim)
        } else {
            grid.text(cols - 9, 0, "○ IDLE", Pal.dim)
        }
        grid.text(1, 1, "spatial acoustic simulator · m0", Pal.faint)
        button(cols - 6, 1, " Q ", .quit, color: Pal.dim)
    }

    private func drawStatus(y: Int, tel: Telemetry, t: Double) {
        let head = model.head
        var x = grid.text(2, y, " HEAD ", Pal.dim)
        if !head.enabled {
            x = grid.text(x, y, "off ", Pal.faint)
        } else if head.active {
            let yaw = head.yaw * 180 / .pi, pitch = head.pitch * 180 / .pi
            x = grid.text(x, y, String(format: "● y%+04.0f° p%+03.0f° ", yaw, pitch), Pal.text)
        } else {
            x = grid.text(x, y, Int(t * 2) % 2 == 0 ? "searching… " : "           ", Pal.amber)
        }
        let right = String(format: " DSP %.1f%% · %.0fk ", tel.dspLoad * 100, tel.sampleRate / 1000)
        grid.text(cols - 2 - right.unicodeScalars.count, y, right, Pal.dim)
        _ = x
    }

    private func drawControls() {
        let audio = model.audio
        let head = model.head
        let y = 30
        var x = 1
        x = button(x, y, audio.isPlaying ? " ❚❚ PAUSE " : " ▶ PLAY ", .play,
                   color: audio.isPlaying ? Pal.amber : Pal.bright) + 2
        x = button(x, y, " ⟲ RECENTER ", .recenter, color: Pal.text) + 2
        _ = button(x, y, head.enabled ? " [■] HEAD TRACK " : " [ ] HEAD TRACK ", .headTrack,
                   color: head.enabled ? Pal.text : Pal.dim)

        for (i, s) in SliderID.allCases.enumerated() {
            slider(s, y: 32 + i)
        }
    }

    @discardableResult
    private func button(_ x: Int, _ y: Int, _ label: String, _ control: Control, color: UInt8) -> Int {
        let w = label.unicodeScalars.count + 2
        let bg = pressed == control ? Pal.activeBg : (hovered == control ? Pal.hoverBg : Pal.panelBg)
        grid.fillBg(x, y, w, 1, bg)
        grid.put(x, y, "[", Pal.dim)
        grid.text(x + 1, y, label, hovered == control ? Pal.bright : color)
        grid.put(x + w - 1, y, "]", Pal.dim)
        regions.append(Region(x: x, y: y, w: w, h: 1, control: control))
        return x + w
    }

    private func slider(_ s: SliderID, y: Int) {
        let v = value(of: s)
        let hot = hovered == .slider(s) || pressed == .slider(s)
        grid.text(1, y, s.label, hot ? Pal.bright : Pal.dim)
        let x0 = Self.sliderX, w = Self.sliderW
        grid.put(x0 - 1, y, "▕", Pal.faint)
        grid.put(x0 + w, y, "▏", Pal.faint)
        let eighths = Int((Double(v) * Double(w * 8)).rounded())
        for i in 0..<w {
            let fill = eighths - i * 8
            let color = Pal.ramp(s.ramp, 0.25 + 0.75 * Float(i) / Float(w - 1))
            if fill >= 8 {
                grid.put(x0 + i, y, "█", color)
            } else if fill > 0 {
                grid.put(x0 + i, y, Self.partials[fill], color)
            } else {
                grid.put(x0 + i, y, "·", Pal.faint)
            }
        }
        grid.text(x0 + w + 2, y, String(format: "%3d%%", Int((v * 100).rounded())), hot ? Pal.bright : Pal.text)
        if hot { grid.fillBg(x0, y, w, 1, Pal.panelBg) }
        regions.append(Region(x: x0 - 2, y: y, w: w + 4, h: 1, control: .slider(s)))
    }

    // MARK: Values

    private func value(of s: SliderID) -> Float {
        let a = model.audio
        switch s {
        case .volume: return a.volume
        case .wind: return a.wind
        case .birds: return a.birds
        case .water: return a.water
        case .orbit: return a.orbit
        }
    }

    private func setValue(_ s: SliderID, _ v: Float) {
        let a = model.audio
        switch s {
        case .volume: a.volume = v
        case .wind: a.wind = v
        case .birds: a.birds = v
        case .water: a.water = v
        case .orbit: a.orbit = v
        }
    }

    // MARK: Input

    private func hit(_ x: Double, _ y: Double) -> Control? {
        let cx = Int(floor(x)), cy = Int(floor(y))
        return regions.first { cx >= $0.x && cx < $0.x + $0.w && cy >= $0.y && cy < $0.y + $0.h }?.control
    }

    func hover(x: Double, y: Double) {
        hovered = hit(x, y)
    }

    func pointer(x: Double, y: Double, phase: PointerPhase) {
        switch phase {
        case .drag:
            if pressed == nil { pressed = hit(x, y) }
            if case .slider(let s)? = pressed {
                let v = (x - Double(Self.sliderX)) / Double(Self.sliderW)
                setValue(s, Float(max(0, min(1, v))))
            }
        case .up:
            if let p = pressed, p == hit(x, y) { activate(p) }
            pressed = nil
        }
    }

    private func activate(_ c: Control) {
        switch c {
        case .play:
            model.togglePlay()
        case .recenter:
            model.head.recenter()
            narrator.say("listener re-centered · forward locked", color: Pal.bright)
        case .headTrack:
            model.head.enabled.toggle()
            narrator.say(model.head.enabled ? "head tracking engaged" : "head tracking off", color: Pal.text)
        case .quit:
            NSApplication.shared.terminate(nil)
        case .slider:
            break
        }
    }
}
