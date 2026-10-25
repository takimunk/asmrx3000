import Foundation
import simd

/// How big a prop is drawn (half its extent, in the stage's mapped units) and which way it faces.
struct PropLook {
    enum Face {
        /// Where it goes (a fish).
        case motion
        /// Its front (+x) towards the listener, upright.
        case listener
        /// Its length (x) across the line to the listener (a wave crest, a brook).
        case across
        /// A fixed turn, varied a little per clip.
        case still
    }

    var size: Float
    var face: Face

    /// Effects and phenomena (wind, rain, a far flash) are see-through; things are solid.
    static func ghost(_ p: Prop) -> Float {
        switch p {
        case .wind, .rain, .rainSurface, .sea, .bolt, .marker:
            return ClipPiece.faint
        default:
            return 0
        }
    }

    static func of(_ p: Prop) -> PropLook {
        switch p {
        case .reef: return PropLook(size: 1.1, face: .still)
        case .coral: return PropLook(size: 0.6, face: .still)
        case .fish: return PropLook(size: 0.5, face: .motion)
        case .rainSurface: return PropLook(size: 0.5, face: .still)
        case .wind: return PropLook(size: 0.8, face: .across)
        case .rain: return PropLook(size: 0.55, face: .still)
        case .sea: return PropLook(size: 1.3, face: .across)
        case .bolt: return PropLook(size: 0.7, face: .listener)
        case .brook: return PropLook(size: 0.95, face: .across)
        case .bead: return PropLook(size: 0.17, face: .still)
        case .singingBowl: return PropLook(size: 0.4, face: .still)
        case .tingsha: return PropLook(size: 0.22, face: .listener)
        case .marker: return PropLook(size: 0.13, face: .still)
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
        let look = PropLook.of(prop)
        let pulse = 1 + 0.08 * level
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

        case .fish:
            put(PropMeshes.fish, root * M4.rotZ(0.12 * sin(t * 2.6 + ph)))

        case .bead:
            // A solid bead in a faint hoop of glow.
            put(PropMeshes.bead, root)
            put(PropMeshes.beadGlow, root * M4.rotZ(t * 0.6 + ph), ghost: ClipPiece.veryFaint)

        case .marker:
            put(PropMeshes.mesh(prop), root * M4.rotZ(t * 0.6 + ph))

        default:
            put(PropMeshes.mesh(prop), root)
        }
        return s * 1.1
    }

    // MARK: Scenery

    /// The still things of the scene: its place (`ClipSet`: the body scan's mat).
    static func scenery(scene: SceneInfo) -> [ClipPiece] {
        ClipSet.of(scene).scenery(groundZ: scene.groundZ)
    }
}
