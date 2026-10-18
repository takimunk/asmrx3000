import CAsmr
import simd

/// How each scene bird is drawn: its size, the wingbeat of its species' flight style, banking
/// into turns, pitching with climbs and dives, folded wings in a finch's bound, sitting on the
/// ground while it forages. Reads `asmr_get_birds` once per frame (docs/director.md, "Life
/// model"); `SceneRenderer` asks for one `Pose` per bird object. Birds without telemetry
/// (flight mode streams its own) get a generic flapping pose.
final class BirdFlight {
    struct Pose {
        /// Model matrix: position, heading, pitch, bank, scale.
        var model: simd_float4x4
        /// Wing angle about the body axis (`Instance.anim.x`).
        var wing: Float
        /// Wings spread (the flying mesh) or folded.
        var spread: Bool
        /// Sitting on the ground (no perch to draw under it).
        var onGround: Bool
    }

    private var raw = [AsmrBirdInfo](repeating: AsmrBirdInfo(), count: 64)
    private var info: [UInt32: AsmrBirdInfo] = [:]
    private var phase: [UInt32: Float] = [:]
    private var bank: [UInt32: Float] = [:]
    private var pitch: [UInt32: Float] = [:]
    private var lastHeading: [UInt32: Float] = [:]
    private var dt: Float = 1 / 60
    private var lastTime: Float = -1

    /// Body size relative to the songbird model, by `ASMR_SPECIES_*`.
    private static let size: [Float] = [0.85, 0.8, 1.05, 0.85, 1.6, 1.25, 1.45, 0.9, 1.1, 0.95, 2.0, 1.9]

    /// Drawn wingbeat (Hz, slower than life so it reads at 60 fps) and amplitude, by `ASMR_FLIGHT_*`.
    private static let beat: [(hz: Float, amp: Float)] = [
        (9, 0.9),    // bounding
        (8, 0.8),    // direct
        (6, 0.7),    // swooping
        (5, 0.95),   // clatter
        (2.5, 0.7),  // silent (owl)
        (2.2, 0.6),  // soaring
        (11, 0.6),   // hovering
    ]

    func update(_ audio: AudioEngine, time: Float) {
        if lastTime >= 0 { dt = min(max(time - lastTime, 1 / 240), 0.1) }
        lastTime = time
        let n = audio.readBirds(&raw)
        info.removeAll(keepingCapacity: true)
        for b in raw.prefix(n) { info[b.id] = b }
    }

    func pose(id: UInt32, at p: SIMD3<Float>, heading h: Float, moving: Bool, ground: Float, time: Float, scale: Float) -> Pose {
        guard let b = info[id] else {
            // No telemetry: the old generic look.
            let wing: Float = moving ? sin(time * 14 + Float(id)) * 0.85 : 0
            return Pose(model: Self.matrix(p, yaw: h, pitch: 0, bank: 0, scale: scale), wing: wing, spread: moving, onGround: false)
        }
        let s = scale * Self.size[min(Int(b.species), Self.size.count - 1)]
        let style = Self.beat[min(Int(b.flight), Self.beat.count - 1)]
        let airborne = moving || b.state == UInt32(ASMR_BIRD_FLYING) || b.state == UInt32(ASMR_BIRD_SOARING)
            || b.state == UInt32(ASMR_BIRD_SONG_FLIGHT) || b.state == UInt32(ASMR_BIRD_CIRCLING)
        let onGround = !airborne && p.z < ground + 0.35
        var pos = p
        if onGround { pos.z = ground + 0.05 * s }

        // Wings: beat while the engine flaps them, hold them in a glide, fold them in a bound.
        var ph = phase[id] ?? Float(id) * 0.7
        var wing: Float = 0
        var spread = airborne
        if airborne {
            if b.flap > 0.02 {
                ph += dt * style.hz * (0.8 + 0.4 * b.flap) * 2 * .pi
                wing = style.amp * min(1, 0.4 + b.flap) * sin(ph)
            } else if b.flight == UInt32(ASMR_FLIGHT_BOUNDING) || b.flight == UInt32(ASMR_FLIGHT_HOVERING) {
                spread = false  // the closed-wing part of a bound (or a lark's plummet)
            } else {
                // Gliding: wings held with a slight dihedral (a gull's shallow M, an owl's flat glide).
                wing = b.flight == UInt32(ASMR_FLIGHT_SOARING) ? -0.08 : 0.12
            }
        } else if b.flap > 0.1 {
            // A wing flick on the ground (a hop) or on the perch.
            spread = true
            ph += dt * 12 * 2 * .pi
            wing = 0.5 * sin(ph)
        }
        phase[id] = ph.truncatingRemainder(dividingBy: 2 * .pi)

        // Attitude: pitch with the climb, bank into the turn (from the heading rate).
        let v = SIMD3(b.vx, b.vy, b.vz)
        let horiz = simd_length(SIMD2(v.x, v.y))
        var pitchTarget: Float = 0, bankTarget: Float = 0
        if airborne && horiz + abs(v.z) > 0.3 {
            pitchTarget = max(-0.6, min(0.6, atan2(v.z, max(horiz, 0.1))))
            if let lh = lastHeading[id] {
                var dh = h - lh
                if dh > .pi { dh -= 2 * .pi } else if dh < -.pi { dh += 2 * .pi }
                bankTarget = max(-0.7, min(0.7, -dh / dt * horiz * 0.08))
            }
        }
        lastHeading[id] = h
        let k = min(1, dt / 0.15)
        let pt = (pitch[id] ?? 0) + (pitchTarget - (pitch[id] ?? 0)) * k
        let bk = (bank[id] ?? 0) + (bankTarget - (bank[id] ?? 0)) * k
        pitch[id] = pt
        bank[id] = bk
        return Pose(model: Self.matrix(pos, yaw: h, pitch: pt, bank: bk, scale: s), wing: wing, spread: spread, onGround: onGround)
    }

    /// Translate · yaw (z) · pitch (nose up = +) · bank (roll about the body axis) · scale.
    static func matrix(_ t: SIMD3<Float>, yaw: Float, pitch: Float, bank: Float, scale: Float) -> simd_float4x4 {
        let q = simd_quatf(angle: yaw, axis: SIMD3(0, 0, 1)) * simd_quatf(angle: -pitch, axis: SIMD3(0, 1, 0))
            * simd_quatf(angle: bank, axis: SIMD3(1, 0, 0))
        var m = simd_float4x4(q)
        m.columns.0 *= scale
        m.columns.1 *= scale
        m.columns.2 *= scale
        m.columns.3 = SIMD4(t, 1)
        return m
    }
}
