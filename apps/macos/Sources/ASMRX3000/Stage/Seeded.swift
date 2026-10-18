/// A tiny deterministic generator (SplitMix64) for scenery that must look the same every launch.
struct SeededGenerator {
    private var state: UInt64

    init(_ seed: UInt64) { state = seed }

    /// Uniform in 0..<1.
    mutating func next() -> Float {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        z ^= z >> 31
        return Float(z >> 40) / Float(1 << 24)
    }
}
