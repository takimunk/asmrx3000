import Foundation
import simd

/// How big a prop is drawn (half its extent, in the stage's mapped units) and which way it faces.
struct PropLook {
    enum Face {
        /// Where it goes (ships, fish, a skater).
        case motion
        /// Its front (+x) towards the listener, upright.
        case listener
        /// Its front straight at the listener, up or down too (a vent overhead).
        case toward
        /// Its length (x) across the line to the listener (a wave crest, a shore).
        case across
        /// A fixed turn, varied a little per clip.
        case still
    }

    var size: Float
    var face: Face

    /// Effects and phenomena (wind, rain, cracks, a far flash) are see-through; things are solid.
    static func ghost(_ p: Prop) -> Float {
        switch p {
        case .wind, .rain, .rainGlass, .rainSurface, .plasma, .sea, .surf, .bolt, .iceStar, .zigzag, .boom, .marker, .bridge, .arches,
             .stream, .ripples, .splash, .flames, .sparks, .steam:
            return ClipPiece.faint
        default:
            return 0
        }
    }

    static func of(_ p: Prop) -> PropLook {
        switch p {
        case .freighter: return PropLook(size: 1.3, face: .motion)
        case .mediumShip: return PropLook(size: 0.85, face: .motion)
        case .shuttle: return PropLook(size: 0.6, face: .motion)
        case .convoy: return PropLook(size: 1.1, face: .motion)
        case .station: return PropLook(size: 1.4, face: .still)
        case .fan: return PropLook(size: 0.5, face: .toward)
        case .pump: return PropLook(size: 0.6, face: .listener)
        case .panel: return PropLook(size: 0.45, face: .toward)
        case .rcs: return PropLook(size: 0.5, face: .listener)
        case .clamp: return PropLook(size: 0.55, face: .listener)
        case .hatch: return PropLook(size: 0.55, face: .toward)
        case .robotArm: return PropLook(size: 0.65, face: .motion)
        case .dish: return PropLook(size: 0.55, face: .listener)
        case .console: return PropLook(size: 0.5, face: .listener)
        case .plasma: return PropLook(size: 2.8, face: .still)
        case .reef: return PropLook(size: 1.1, face: .still)
        case .coral: return PropLook(size: 0.6, face: .still)
        case .fish: return PropLook(size: 0.5, face: .motion)
        case .rainSurface: return PropLook(size: 0.5, face: .still)
        case .clockwork: return PropLook(size: 0.45, face: .listener)
        case .wind: return PropLook(size: 0.8, face: .across)
        case .rainGlass: return PropLook(size: 0.5, face: .listener)
        case .rain: return PropLook(size: 0.55, face: .still)
        case .sea: return PropLook(size: 1.3, face: .across)
        case .surf: return PropLook(size: 0.9, face: .across)
        case .buoy: return PropLook(size: 0.6, face: .still)
        case .foghorn: return PropLook(size: 0.8, face: .listener)
        case .bolt: return PropLook(size: 0.7, face: .listener)
        case .pane: return PropLook(size: 0.42, face: .listener)
        case .stairs: return PropLook(size: 0.55, face: .still)
        case .track, .bridge: return PropLook(size: 1, face: .still)
        case .bogie: return PropLook(size: 0.6, face: .still)
        case .crossing: return PropLook(size: 0.55, face: .listener)
        case .train: return PropLook(size: 0.75, face: .motion)
        case .arches: return PropLook(size: 1, face: .still)
        case .teaGlass: return PropLook(size: 0.36, face: .listener)
        case .vent: return PropLook(size: 0.42, face: .toward)
        case .horn: return PropLook(size: 0.5, face: .listener)
        case .lampPosts: return PropLook(size: 0.7, face: .still)
        case .treeLine: return PropLook(size: 1.5, face: .listener)
        case .iceStar: return PropLook(size: 0.9, face: .still)
        case .zigzag: return PropLook(size: 1.0, face: .motion)
        case .boom: return PropLook(size: 1.0, face: .still)
        case .owl: return PropLook(size: 0.5, face: .listener)
        case .skater: return PropLook(size: 0.5, face: .motion)
        case .kettle: return PropLook(size: 0.45, face: .listener)
        case .brazier: return PropLook(size: 0.42, face: .still)
        case .cloth: return PropLook(size: 0.3, face: .still)
        case .ladle: return PropLook(size: 0.4, face: .still)
        case .whisk: return PropLook(size: 0.25, face: .still)
        case .host: return PropLook(size: 0.55, face: .listener)
        case .teaBowl: return PropLook(size: 0.27, face: .still)
        case .eaves: return PropLook(size: 0.6, face: .across)
        case .basin: return PropLook(size: 0.6, face: .listener)
        case .shishi: return PropLook(size: 0.55, face: .across)
        case .chime: return PropLook(size: 0.45, face: .still)
        case .templeBell: return PropLook(size: 0.9, face: .listener)
        case .bead: return PropLook(size: 0.17, face: .still)
        case .singingBowl: return PropLook(size: 0.4, face: .still)
        case .tingsha: return PropLook(size: 0.22, face: .listener)
        case .ripples: return PropLook(size: 0.55, face: .listener)
        case .splash: return PropLook(size: 0.3, face: .still)
        case .joinery: return PropLook(size: 0.3, face: .listener)
        case .cleat: return PropLook(size: 0.3, face: .still)
        case .halyard: return PropLook(size: 0.45, face: .still)
        case .chain: return PropLook(size: 0.6, face: .still)
        case .mooredYacht: return PropLook(size: 1.5, face: .still)
        case .gull: return PropLook(size: 0.4, face: .motion)
        case .marker: return PropLook(size: 0.13, face: .still)
        case .pool: return PropLook(size: 0.42, face: .still)
        case .stalactite: return PropLook(size: 0.28, face: .still)
        case .stalagmite: return PropLook(size: 0.32, face: .still)
        case .pebbles: return PropLook(size: 0.3, face: .motion)
        case .stream: return PropLook(size: 0.8, face: .across)
        case .campfire: return PropLook(size: 0.6, face: .still)
        case .flames: return PropLook(size: 0.5, face: .still)
        case .sparks: return PropLook(size: 0.45, face: .still)
        case .embers: return PropLook(size: 0.3, face: .still)
        case .driftLog: return PropLook(size: 0.3, face: .still)
        case .steam: return PropLook(size: 0.26, face: .still)
        case .duneGrass: return PropLook(size: 0.45, face: .still)
        case .curlew: return PropLook(size: 0.55, face: .listener)
        }
    }
}

extension ClipActors {
    /// The pieces of a prop at its mapped place: the mesh posed once per frame (and its moving
    /// parts), copies at each emitter for the spread-out ones. Returns the half size for the
    /// selection brackets.
    func propPieces(_ prop: Prop, clip: ClipInfo, raw: SIMD3<Float>, time t: Float, level: Float,
                    fwd: () -> SIMD3<Float>, out: inout [ClipPiece]) -> Float {
        let p = ClipSpace.map(raw)
        let id = clip.id
        let ph = Float(id % 13) * 0.9
        let pulse = 1 + 0.08 * level
        let look = PropLook.of(prop)
        let s = look.size * pulse
        let ghost = PropLook.ghost(prop)
        func put(_ mesh: Mesh?, _ m: simd_float4x4, ghost g: Float? = nil) {
            if let mesh { out.append(ClipPiece(mesh: mesh, model: m, ghost: g ?? ghost)) }
        }
        /// Turn for a thing at mapped `q`.
        func facing(_ q: SIMD3<Float>) -> simd_float4x4 {
            switch look.face {
            case .motion: return M4.facing(fwd())
            case .listener:
                let d = SIMD3(-q.x, -q.y, 0)
                return simd_length(d) > 1e-3 ? M4.facing(d) : matrix_identity_float4x4
            case .toward: return simd_length(q) > 1e-3 ? M4.facing(-q) : matrix_identity_float4x4
            case .across:
                let d = SIMD3(-q.y, q.x, 0)
                return simd_length(d) > 1e-3 ? M4.facing(d) : matrix_identity_float4x4
            case .still: return M4.rotZ(0.7 * Float(id % 7))
            }
        }
        /// The raw places of the clip's emitters (a ring of `spread` about it), or the clip itself.
        func emitters() -> [SIMD3<Float>] {
            guard clip.emitters > 1, clip.spread > 0 else { return [raw] }
            return (0..<clip.emitters).map { k in
                let a = 2 * Float.pi * Float(k) / Float(clip.emitters)
                return raw + SIMD3(cos(a), sin(a), 0) * clip.spread
            }
        }
        let root = M4.move(p) * facing(p) * M4.scale(s)

        switch prop {
        case .fan:
            put(PropMeshes.fanFrame, root)
            put(PropMeshes.fanBlades, root * M4.rotX(t * 2.4 + ph))

        case .hatch:
            put(PropMeshes.hatch, root)
            put(PropMeshes.hatchWheel, root * M4.rotX(0.6 * sin(t * 0.3 + ph)))

        case .clockwork:
            put(PropMeshes.clockFrame, root)
            put(PropMeshes.gear, root * M4.move(SIMD3(0, 0, 0.1)) * M4.rotY(t * 0.35 + ph))

        case .shishi:
            // Fills slowly (the long end sinking), then tips and knocks back.
            let cycle = (t / 7 + ph).truncatingRemainder(dividingBy: 1)
            let tilt: Float = cycle < 0.88 ? -0.32 + 0.3 * cycle : 0.42 - 0.6 * (cycle - 0.88) / 0.12
            put(PropMeshes.shishiFrame, root)
            put(PropMeshes.shishiTube, root * M4.rotY(-tilt))

        case .chime:
            put(PropMeshes.chime, root * M4.rotX(0.22 * sin(t * 1.3 + ph)) * M4.rotY(0.12 * sin(t * 0.9 + ph)))

        case .buoy:
            put(PropMeshes.buoy, root * M4.rotX(0.16 * sin(t * 0.8 + ph)) * M4.rotY(0.11 * sin(t * 0.55 + ph)))

        case .bolt:
            put(PropMeshes.bolt, M4.move(p) * facing(p) * M4.scale(look.size * (0.75 + 0.6 * level)))

        case .wind:
            // Streamers at each emitter, turning about their length so they seem to stream.
            for (k, e) in emitters().enumerated() {
                let q = ClipSpace.map(e)
                put(PropMeshes.wind, M4.move(q) * facing(q) * M4.rotZ(0.3 * sin(Float(k) * 1.7))
                    * M4.rotX(t * 2.2 + Float(k)) * M4.scale(s))
            }

        case .rain:
            for (k, e) in emitters().enumerated() {
                let fall = (t * 1.5 + Float(k) * 0.37).truncatingRemainder(dividingBy: 1) * 0.4
                put(PropMeshes.rain, M4.move(ClipSpace.map(e)) * M4.rotZ(Float(k)) * M4.scale(s) * M4.move(SIMD3(0, 0, -fall)))
            }

        case .sea:
            for (k, e) in emitters().enumerated() {
                let q = ClipSpace.map(e)
                put(PropMeshes.crest, M4.move(q + SIMD3(0, 0, 0.1 * sin(t * 0.8 + Float(k) * 1.3))) * facing(q) * M4.scale(s))
            }

        case .reef:
            put(PropMeshes.reef, root)
            for (k, e) in emitters().enumerated() where clip.emitters > 1 {
                let q = ClipSpace.map(e)
                put(PropMeshes.coral, M4.move(q) * M4.rotZ(Float(k) * 2.1) * M4.scale(0.55 * pulse))
            }

        case .treeLine:
            for e in emitters() + (clip.emitters > 1 ? [raw] : []) {
                let q = ClipSpace.map(e)
                put(PropMeshes.treeLine, M4.move(q) * facing(q) * M4.scale(s))
            }

        case .rainSurface:
            // Rain on the water's skin overhead: drops falling on it and rings spreading from them.
            for (k, e) in emitters().enumerated() {
                let q = ClipSpace.map(e)
                let fall = (t * 0.9 + Float(k) * 0.37).truncatingRemainder(dividingBy: 1) * 0.12
                put(PropMeshes.droplets, M4.move(q) * M4.scale(s) * M4.move(SIMD3(0, 0, -fall)))
                for j in 0..<2 {
                    let age = (t * 0.5 + Float(k) * 0.29 + Float(j) * 0.5).truncatingRemainder(dividingBy: 1)
                    put(PropMeshes.hoop, M4.move(q) * M4.scale(SIMD3(repeating: s * (0.2 + 1.2 * age))))
                }
            }
            return 0.6

        case .track, .bridge:
            if placed[id] == nil, let m = (prop == .track ? Self.track(raw) : Self.bridge(raw)).warped(ClipSpace.map).build() {
                placed[id] = m
            }
            put(placed[id], matrix_identity_float4x4)
            return 0.8

        case .arches:
            // Arches of the tunnel coming at the train and past it; a ring about the way stays
            // a ring when mapped (every point at the same distance), so each is exact.
            let radius = max(clip.spread, 2)
            for k in 0..<6 {
                let x = 21 - (t * 14 + Float(k) * 7).truncatingRemainder(dividingBy: 42)
                let r = (x * x + radius * radius).squareRoot()
                let f = ClipSpace.radius(r) / r
                put(PropMeshes.arch, M4.move(SIMD3(f * x, 0, f * raw.z)) * M4.scale(f * radius))
            }
            return 0.6

        case .boom:
            // Rings spreading under the ice.
            for k in 0..<3 {
                let age = (t * 0.22 + Float(k) / 3 + ph).truncatingRemainder(dividingBy: 1)
                put(PropMeshes.hoop, M4.move(p) * M4.scale(SIMD3(repeating: s * (0.2 + 1.1 * age))))
            }

        case .lampPosts:
            // Lamps lean over the track, whichever side the platform is on.
            let side: Float = raw.y < 0 ? -1 : 1
            put(PropMeshes.lampPosts, M4.move(p) * M4.scale(SIMD3(s, s * side, s)))

        case .bogie:
            put(PropMeshes.bogie, M4.move(p) * M4.scale(s))

        case .ripples:
            // Wavelets at the hull rising and falling with the roll.
            put(PropMeshes.ripples, root * M4.move(SIMD3(0, 0, 0.1 + 0.08 * sin(t * 1.09 + ph))) * M4.scale(SIMD3(1, 1, 1 + 0.2 * level)))

        case .chain, .cleat:
            // Along the boat (+x to the bow), as the set's chain runs.
            put(PropMeshes.mesh(prop), M4.move(p) * M4.scale(s))

        case .halyard:
            // The stretch of mast stays; the line swings off it with the roll.
            let up = M4.move(p) * M4.rotZ(.pi / 2) * M4.scale(s)
            put(PropMeshes.halyardMast, up)
            put(PropMeshes.halyardLine, up * M4.move(SIMD3(0, 0, 1)) * M4.rotY(0.25 * sin(t * 1.09 + ph)) * M4.move(SIMD3(0, 0, -1)))

        case .mooredYacht:
            // Head to the wind, rocking at its mooring.
            put(PropMeshes.mooredYacht, M4.move(p) * M4.rotZ(0.4 + 0.1 * Float(id % 3)) * M4.rotX(0.06 * sin(t * 1.1 + ph)) * M4.scale(s))

        case .gull:
            // Gliding on long wings, now and then a slow beat.
            let beat = 0.35 * sin(t * 2.4 + ph) * max(sin(t * 0.3 + ph), 0) + 0.12
            put(PropMeshes.gullBody, root)
            put(PropMeshes.gullWing, root * M4.move(SIMD3(0.05, 0.05, 0.03)) * M4.rotX(beat))
            put(PropMeshes.gullWing, root * M4.move(SIMD3(0.05, -0.05, 0.03)) * M4.scale(SIMD3(1, -1, 1)) * M4.rotX(beat))

        case .skater:
            put(PropMeshes.skater, root * M4.rotX(0.18 * sin(t * 1.1 + ph)))

        case .fish:
            put(PropMeshes.fish, root * M4.rotZ(0.12 * sin(t * 2.6 + ph)))

        case .pool:
            // The pool, and rings spreading on it while a drop sounds.
            put(PropMeshes.pool, root)
            if level > 0.02 {
                for k in 0..<2 {
                    let age = (t * 0.7 + Float(k) * 0.5 + ph).truncatingRemainder(dividingBy: 1)
                    put(PropMeshes.hoop, M4.move(p + SIMD3(0, 0, 0.01)) * M4.scale(SIMD3(repeating: s * (0.12 + 0.55 * age))),
                        ghost: ClipPiece.faint)
                }
            }

        case .stalactite:
            // A drop forms at the tip and falls into the basin, every ~6.5 s.
            put(PropMeshes.stalactite, root)
            let u = (t / 6.5 + ph).truncatingRemainder(dividingBy: 1)
            let z: Float = u < 0.8 ? 1.85 : 1.85 * (1 - (u - 0.8) / 0.2) * (1 - (u - 0.8) / 0.2)
            put(ClipProps.sphere, root * M4.move(SIMD3(0, 0, max(z, 0.05))) * M4.scale(SIMD3(0.07, 0.07, 0.1)))

        case .pebbles:
            put(PropMeshes.pebbles, root * M4.rotY(t * 2.2 + ph))

        case .stream:
            for (k, e) in emitters().enumerated() {
                let q = ClipSpace.map(e)
                let drift = (t * 0.25 + Float(k) * 0.3).truncatingRemainder(dividingBy: 1) * 0.4 - 0.2
                put(PropMeshes.stream, M4.move(q) * facing(q) * M4.scale(s) * M4.move(SIMD3(drift, 0, 0)))
            }

        case .bead:
            // A solid bead in a faint hoop of glow.
            put(PropMeshes.bead, root)
            put(PropMeshes.beadGlow, root * M4.rotZ(t * 0.6 + ph), ghost: ClipPiece.veryFaint)

        case .flames:
            // Taller as the fire sounds louder, licking upward at the flame's flicker rate.
            let lick = 0.82 + 0.5 * level + 0.08 * sin(t * 9 + ph) + 0.05 * sin(t * 13.7 + 2 * ph)
            put(PropMeshes.flames, M4.move(p) * M4.rotZ(0.3 * sin(t * 0.7 + ph)) * M4.scale(SIMD3(s, s, s * lick)))

        case .sparks:
            let rise = (t * 0.35 + ph).truncatingRemainder(dividingBy: 1) * 0.13
            put(PropMeshes.sparks, root * M4.move(SIMD3(0, 0, rise)) * M4.rotZ(t * 0.2))

        case .steam:
            put(PropMeshes.steam, root * M4.rotZ(0.25 * sin(t * 0.6 + ph)) * M4.scale(SIMD3(1, 1, 0.7 + 0.5 * level)))

        case .duneGrass:
            // A tuft at each emitter, swaying in the breeze off the sea.
            for (k, e) in emitters().enumerated() {
                let q = ClipSpace.map(e)
                let sway = 0.06 + 0.1 * level
                put(PropMeshes.duneGrass, M4.move(q) * M4.rotZ(Float(k) * 1.9) * M4.rotY(sway * sin(t * 1.4 + Float(k) * 1.3)) * M4.scale(s))
            }

        case .plasma, .marker:
            put(PropMeshes.mesh(prop), root * M4.rotZ(t * (prop == .plasma ? 0.05 : 0.6) + ph))

        default:
            put(PropMeshes.mesh(prop), root)
        }
        return s * 1.1
    }

    // MARK: Things built in place

    /// Rails under the floor from far behind to far ahead, sleepers near.
    static func track(_ c: SIMD3<Float>) -> MeshBuilder {
        var b = MeshBuilder()
        let xs: [Float] = [-80, -50, -32, -22, -15, -10, -7, -5, -3.5, -2.4, -1.5, -0.7, 0, 0.7, 1.5, 2.4, 3.5, 5, 7, 10, 15, 22, 32, 50, 80]
        for y: Float in [-0.72, 0.72] {
            b.path(xs.map { c + SIMD3($0, y, 0.05) }, 0.035, segs: 4, part: Part.stone)
        }
        for x in stride(from: Float(-9), through: 9, by: 0.65) {
            b.box(c + SIMD3(x, 0, -0.05), SIMD3(0.2, 1.9, 0.08), part: Part.section)
        }
        return b
    }

    /// Two truss girders below the train.
    static func bridge(_ c: SIMD3<Float>) -> MeshBuilder {
        var b = MeshBuilder()
        let half: Float = 14, step: Float = 1.75
        for y: Float in [-1.7, 1.7] {
            let lo: Float = -0.45, hi: Float = 0.45
            b.path(stride(from: -half, through: half, by: step).map { c + SIMD3($0, y, lo) }, 0.05, segs: 4, part: Part.stone)
            b.path(stride(from: -half, through: half, by: step).map { c + SIMD3($0, y, hi) }, 0.05, segs: 4, part: Part.stone)
            var x = -half
            var k = 0
            while x < half - 0.01 {
                b.rod(c + SIMD3(x, y, lo), c + SIMD3(x, y, hi), 0.035, segs: 4, part: Part.stone)
                let up = k % 2 == 0
                b.rod(c + SIMD3(x, y, up ? lo : hi), c + SIMD3(x + step, y, up ? hi : lo), 0.03, segs: 4, part: Part.stone)
                x += step
                k += 1
            }
        }
        return b
    }

    // MARK: Scenery

    /// The still things of the scene: its place (`ClipSet`: the lantern room, the compartment,
    /// the lake…). An unknown scene with ice sounds still gets the ice under it. No seated figure
    /// under the body-scan guide: the user found it ugly; the guide's own path reads the body.
    static func scenery(kinds: [ClipKind], scene: SceneInfo) -> [ClipPiece] {
        let set = ClipSet.of(scene)
        var out = set.scenery(groundZ: scene.groundZ)
        let ice = kinds.contains { k in
            if case .prop(let p) = k { return p == .iceStar || p == .zigzag || p == .boom }
            return false
        }
        if set == .none, ice, scene.look == .void, let m = iceSheet(z: scene.groundZ) { out.append(ClipPiece(mesh: m, model: matrix_identity_float4x4)) }
        return out
    }

    /// The frozen lake as far as it carries, mapped (it bows down near the listener).
    static func iceSheet(z: Float) -> Mesh? {
        var pos: [SIMD3<Float>] = []
        var tri: [UInt32] = []
        let segs = 48
        var radii: [Float] = []
        var r: Float = 0.5
        while r < 900 { radii.append(r); r *= 1.3 }
        pos.append(SIMD3(0, 0, z))
        for rr in radii {
            for k in 0..<segs {
                let a = 2 * Float.pi * Float(k) / Float(segs)
                pos.append(SIMD3(rr * cos(a), rr * sin(a), z))
            }
        }
        func at(_ r: Int, _ k: Int) -> UInt32 { UInt32(1 + r * segs + k % segs) }
        for k in 0..<segs { tri += [0, at(0, k), at(0, k + 1)] }
        for r in 0..<(radii.count - 1) {
            for k in 0..<segs { tri += [at(r, k), at(r + 1, k), at(r, k + 1), at(r, k + 1), at(r + 1, k), at(r + 1, k + 1)] }
        }
        var b = MeshBuilder()
        b.smooth(pos, tri, part: Part.ground)
        return b.warped(ClipSpace.map).build()
    }
}
