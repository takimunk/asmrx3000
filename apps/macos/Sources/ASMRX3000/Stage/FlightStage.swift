import CAsmr
import simd

/// What the scene needs to draw flight mode (ASMR_PARAM_FLIGHT): the streamed landscape is
/// reported listener-relative, so the diorama stays centred on the herm while the relief,
/// trees and water pass under it.
struct FlightView {
    /// Cross-fade 0 (scene) .. 1 (flying); objects switch at 0.5.
    var mix: Float = 0
    /// Ears above the ground, m: the ground sits that far below the herm.
    var altitude: Float = 1.7
    /// Distance flown, m.
    var odometer: Float = 0
    var heading: Float = 0
    /// Where the listener is in the landscape, absolute heights, the ground's look and the
    /// precipitation type (engine's AsmrFlightFrame).
    var frame = AsmrFlightFrame()
    var framed = false
    /// Nearest river crossing the disc, as the shader's river points (x, y, z, level): its
    /// ripples.
    var river: [SIMD4<Float>] = []

    var active: Bool { mix > 0.5 }
    /// How far the ground sits below the diorama's usual ground (camera framing).
    var drop: Float { active ? max(altitude - 1.7, 0) : 0 }
}

extension Stage {
    /// Reads the flight telemetry, follows the landscape (chunked relief) and the river
    /// nearest the herm.
    func readFlight() {
        flight.mix = telemetry.flight
        flight.altitude = telemetry.altitude
        flight.odometer = telemetry.odometer
        flight.heading = telemetry.heading
        flight.river.removeAll(keepingCapacity: true)
        if flight.mix > 0, let f = audio.readFlightFrame() {
            flight.frame = f
            flight.framed = true
            terrain.update(f)
        }
        guard flight.active else { return }
        var pts: [SIMD3<Float>] = []
        var bestRiver: (Float, [SIMD4<Float>])?
        let reach = FlightGround.reach
        for o in objects where o.kind == UInt32(ASMR_OBJECT_RIVER) {
            audio.readObjectPath(o.id, into: &pts)
            let near = pts.filter { simd_length(SIMD2($0.x, $0.y)) < reach + 30 }.sorted { $0.y < $1.y }
            guard near.count >= 2 else { continue }
            let d = near.map { simd_length(SIMD2($0.x, $0.y)) }.min() ?? .infinity
            if d < reach, bestRiver == nil || d < bestRiver!.0 {
                bestRiver = (d, near.map { SIMD4($0.x, $0.y, $0.z + 0.72, 0.5) })
            }
        }
        if let r = bestRiver?.1 {
            flight.river = r.count > 8 ? (0..<8).map { r[$0 * (r.count - 1) / 7] } : r
        }
    }

    /// Ground under a travel-frame point in flight, relative to the ears.
    func flightGround(_ x: Float, _ y: Float) -> Float {
        terrain.height(x, y) ?? -flight.altitude
    }
}

/// Flight-mode constants and the boulder model.
enum FlightGround {
    /// Streamed things (trees, birds, boulders) are drawn out to here; beyond it the relief
    /// alone runs on into the haze.
    static let reach: Float = 90

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
