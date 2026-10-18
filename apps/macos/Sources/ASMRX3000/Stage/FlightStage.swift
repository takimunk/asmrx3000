import CAsmr
import simd

/// What the scene needs to draw flight mode (ASMR_PARAM_FLIGHT): the streamed landscape is
/// reported listener-relative, so the diorama stays centred on the herm while the ground,
/// trees and water pass under it.
struct FlightView {
    /// Cross-fade 0 (scene) .. 1 (flying); objects switch at 0.5.
    var mix: Float = 0
    /// Ears above the ground, m: the diorama sinks by `drop` so the herm flies at that height.
    var altitude: Float = 1.7
    /// Distance flown, m.
    var odometer: Float = 0
    /// The listener's place and heading in the landscape (wrapping at 4096 m): the ground
    /// relief is anchored to the world, so it slides and turns under the herm as the course
    /// does. The view itself stays in the travel frame (+x = where you fly).
    var heading: Float = 0
    var worldX: Float = 0
    var worldY: Float = 0
    /// Nearest river crossing the disc, as the shader's river points (x, y, z, level).
    var river: [SIMD4<Float>] = []
    /// Lake shores within reach (listener-relative outlines).
    var lakes: [[SIMD2<Float>]] = []

    var active: Bool { mix > 0.5 }
    /// How far the ground sits below the diorama's usual ground.
    var drop: Float { active ? max(altitude - 1.7, 0) : 0 }
}

extension Stage {
    /// Reads the flight telemetry and the shapes of rivers and lakes near the disc.
    func readFlight() {
        flight.mix = telemetry.flight
        flight.altitude = telemetry.altitude
        flight.odometer = telemetry.odometer
        flight.heading = telemetry.heading
        flight.worldX = telemetry.worldX
        flight.worldY = telemetry.worldY
        flight.river.removeAll(keepingCapacity: true)
        flight.lakes.removeAll(keepingCapacity: true)
        guard flight.active else { return }
        var pts: [SIMD3<Float>] = []
        var bestRiver: (Float, [SIMD4<Float>])?
        let reach = FlightGround.reach
        for o in objects where o.kind == UInt32(ASMR_OBJECT_RIVER) || o.kind == UInt32(ASMR_OBJECT_LAKE) {
            audio.readObjectPath(o.id, into: &pts)
            guard pts.count >= 2 else { continue }
            if o.kind == UInt32(ASMR_OBJECT_LAKE) {
                if pts.contains(where: { simd_length(SIMD2($0.x, $0.y)) < reach + 40 }) {
                    flight.lakes.append(pts.map { SIMD2($0.x, $0.y) })
                }
                continue
            }
            // The river as x(y) across the disc (rivers cross the flight path), in the
            // diorama's frame (bed at the usual river height).
            let near = pts.filter { simd_length(SIMD2($0.x, $0.y)) < reach + 30 }.sorted { $0.y < $1.y }
            guard near.count >= 2 else { continue }
            // The shader draws a river as x(y), i.e. one that crosses the course; one running
            // along it (after a turn) is heard but not drawn.
            let spanX = (near.map(\.x).max() ?? 0) - (near.map(\.x).min() ?? 0)
            let spanY = near.last!.y - near.first!.y
            guard spanY > 0.8 * spanX else { continue }
            let d = near.map { abs($0.x) }.min() ?? .infinity
            if d < reach, bestRiver == nil || d < bestRiver!.0 {
                bestRiver = (d, near.map { SIMD4($0.x, $0.y, -1.2, 0.5) })
            }
        }
        if let r = bestRiver?.1 {
            flight.river = r.count > 8 ? (0..<8).map { r[$0 * (r.count - 1) / 7] } : r
        }
    }
}

/// The ground disc in flight: the relief scrolls with the distance flown, rivers and lakes
/// cut through it. Coarser than the scene's terrain because it is rebuilt every frame.
enum FlightGround {
    static let rings = 44
    static let farRings = 24
    static let segments = 132
    /// Streamed things are drawn out to here; beyond it the haze has them.
    static let reach: Float = 70

    /// Relief at a listener-relative point (the world slides by under the herm).
    static func height(_ x: Float, _ y: Float, frame f: FlightView) -> Float {
        let (s, c) = (sin(f.heading), cos(f.heading))
        let wx = f.worldX + c * x - s * y
        let wy = f.worldY + s * x + c * y
        return Terrain.height(wx, wy) + 0.07 * sin(wx * 2.3 + 0.7) * sin(wy * 1.9)
    }

    static func inside(_ p: SIMD2<Float>, _ poly: [SIMD2<Float>]) -> Bool {
        var c = false
        var j = poly.count - 1
        for i in 0..<poly.count {
            let a = poly[i], b = poly[j]
            if (a.y > p.y) != (b.y > p.y), p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x { c.toggle() }
            j = i
        }
        return c
    }

    static func build(_ f: FlightView) -> Mesh? {
        func surface(_ x: Float, _ y: Float) -> (Float, Float) {
            let base = height(x, y, frame: f)
            for lake in f.lakes where inside(SIMD2(x, y), lake) {
                return (-1.95, Part.water)
            }
            guard !f.river.isEmpty else { return (base, Part.ground) }
            let rv = Stage.riverAt(y, f.river)
            let dist = abs(x - rv.x)
            let wl = Terrain.waterLevel(rv.y)
            let t = min(max((dist - 0.5) / 2.0, 0), 1)
            let bank = t * t * (3 - 2 * t)
            let bed = wl - 0.42 + 0.12 * dist
            let ground = bed + (base - bed) * bank
            if dist < 2.2 && ground < wl - 0.02 { return (wl, Part.water) }
            return (max(ground, wl + 0.05), Part.ground)
        }
        // Rimless, out to the haze (GroundMesh); coarser than the scene's, as it is rebuilt every frame.
        return GroundMesh.build(segments: segments, inner: rings, outer: farRings, surface: surface)
    }

    /// A unit boulder (radius 1, resting on z = 0), stone.
    static func makeBoulder() -> Mesh? {
        var v: [MeshVertex] = []
        var idx: [UInt32] = []
        let rings = 6, segs = 9
        func bump(_ a: Float, _ b: Float) -> Float { 1 + 0.16 * sin(a * 3.1 + 0.4) * cos(b * 2.3) + 0.08 * sin(a * 7 + b * 5) }
        for i in 0...rings {
            let b = Float.pi * (Float(i) / Float(rings) - 0.5)
            for j in 0..<segs {
                let a = 2 * Float.pi * Float(j) / Float(segs)
                let r = bump(a, b)
                let n = SIMD3(cos(b) * cos(a), cos(b) * sin(a), sin(b))
                // Flattened, half sunk in the ground.
                v.append(MeshVertex(SIMD3(n.x * r, n.y * r, 0.35 + n.z * r * 0.7), n, Part.stone))
            }
        }
        for i in 0..<rings {
            for j in 0..<segs {
                let a = UInt32(i * segs + j), b = UInt32(i * segs + (j + 1) % segs)
                let c = a + UInt32(segs), d = b + UInt32(segs)
                idx += [a, b, c, b, d, c]
            }
        }
        return Mesh.make(v, idx)
    }
}
