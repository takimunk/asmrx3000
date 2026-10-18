import simd

/// Ground height of the diorama away from the river. Must match `terrain()` in Shaders.swift.
enum Terrain {
    /// Radius of the ground disc the scene sits on.
    static let radius: Float = 13

    /// The land beyond the play area: the same ground, rising into low hills towards the horizon.
    static func farHeight(_ x: Float, _ y: Float) -> Float {
        let r = (x * x + y * y).squareRoot()
        let t = min(max((r - 15) / 45, 0), 1)
        let hills = 2.2 * sin(x * 0.045 + 0.8) * sin(y * 0.038 + 2.1) + 1.1 * sin(x * 0.11 - y * 0.07 + 0.4)
        return height(x, y) + t * t * (3 - 2 * t) * (hills + 0.9)
    }

    static func height(_ x: Float, _ y: Float) -> Float {
        -1.7 + 0.22 * sin(x * 0.31 + 1.3) * sin(y * 0.23 + 0.4)
            + 0.06 * sin(x * 1.10 + y * 0.70) + 0.04 * sin(y * 1.9 - x * 0.4)
    }

    /// Water surface for a river source at height `z`. Must match `waterLevel()` in the shader.
    static func waterLevel(_ z: Float) -> Float { z - 0.72 }
}

/// Orbits a target in the engine's frame (+x forward, +y left, +z up).
struct OrbitCamera {
    var target = SIMD3<Float>(2.0, 0, -1.4)
    var azimuth: Float = .pi + 0.62
    // Low enough that the ground runs away into the haze, high enough to see over the river.
    var elevation: Float = 0.25
    var distance: Float = 36
    var fovY: Float = 0.58

    var eye: SIMD3<Float> {
        target + distance * SIMD3(cos(elevation) * cos(azimuth), cos(elevation) * sin(azimuth), sin(elevation))
    }

    func viewProjection(aspect: Float) -> simd_float4x4 {
        let e = eye
        let f = simd_normalize(target - e)
        let s = simd_normalize(simd_cross(f, SIMD3(0, 0, 1)))
        let u = simd_cross(s, f)
        let view = simd_float4x4(rows: [
            SIMD4(s.x, s.y, s.z, -simd_dot(s, e)),
            SIMD4(u.x, u.y, u.z, -simd_dot(u, e)),
            SIMD4(-f.x, -f.y, -f.z, simd_dot(f, e)),
            SIMD4(0, 0, 0, 1),
        ])
        let near: Float = 0.5, far: Float = 120
        let ys = 1 / tan(fovY / 2)
        let zs = far / (near - far)
        let proj = simd_float4x4(columns: (
            SIMD4(ys / aspect, 0, 0, 0),
            SIMD4(0, ys, 0, 0),
            SIMD4(0, 0, zs, -1),
            SIMD4(0, 0, zs * near, 0)
        ))
        return proj * view
    }

    /// Ground-plane directions for "away from the camera" and "to the right on screen",
    /// snapped to world axes so arrow-key moves keep coordinates tidy.
    var screenAxes: (forward: SIMD2<Float>, right: SIMD2<Float>) {
        let f = SIMD2(-cos(azimuth), -sin(azimuth))
        let snapped: SIMD2<Float> = abs(f.x) >= abs(f.y) ? SIMD2(f.x.sign == .minus ? -1 : 1, 0)
                                                         : SIMD2(0, f.y.sign == .minus ? -1 : 1)
        return (snapped, SIMD2(snapped.y, -snapped.x))
    }
}

/// Projection into view space (points, y down).
struct Projector {
    let matrix: simd_float4x4
    let size: SIMD2<Float>

    init(camera: OrbitCamera, size: SIMD2<Float>) {
        self.size = size
        matrix = camera.viewProjection(aspect: size.x / max(size.y, 1))
    }

    /// Returns the screen point and clip w (distance-like), or nil behind the camera.
    func project(_ p: SIMD3<Float>) -> (point: SIMD2<Float>, w: Float)? {
        let c = matrix * SIMD4(p, 1)
        guard c.w > 0.05 else { return nil }
        let ndc = SIMD2(c.x, c.y) / c.w
        return (SIMD2((ndc.x * 0.5 + 0.5) * size.x, (0.5 - ndc.y * 0.5) * size.y), c.w)
    }

    /// Screen bounds of a world-space box.
    func bounds(min lo: SIMD3<Float>, max hi: SIMD3<Float>) -> SIMD4<Float>? {
        var mn = SIMD2<Float>(repeating: .infinity), mx = SIMD2<Float>(repeating: -.infinity)
        for i in 0..<8 {
            let p = SIMD3(i & 1 == 0 ? lo.x : hi.x, i & 2 == 0 ? lo.y : hi.y, i & 4 == 0 ? lo.z : hi.z)
            guard let s = project(p) else { return nil }
            mn = simd_min(mn, s.point)
            mx = simd_max(mx, s.point)
        }
        return SIMD4(mn.x, mn.y, mx.x, mx.y)
    }
}
