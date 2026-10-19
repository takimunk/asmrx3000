import Foundation
import CAsmr

private func XCTAssertEqual(_ actual: Double, _ expected: Double, accuracy: Double) {
    precondition(abs(actual - expected) <= accuracy, "expected \(expected), got \(actual)")
}

private func XCTAssertEqual<T: Equatable>(_ actual: T, _ expected: T) {
    precondition(actual == expected, "expected \(expected), got \(actual)")
}

@main
struct AudioStateCheck {
    static func main() throws {
        let checks = AudioStateCheck()
        try checks.testPatchUndoRestoresOriginalValueAndFollower()
        checks.testSnapshotPreservesLatestEditsBeyondOldHistoryLimit()
        checks.testSnapshotPreservesGlideTargetAndPendingObjectPosition()
        checks.testFlightSwitchHasThreePlaces()
        print("4 audio state checks passed")
    }

    private func patch(_ audio: AudioEngine, _ key: String, _ value: Any, glide: Double = 0) -> [String: Any] {
        let data = try! JSONSerialization.data(withJSONObject: [
            "glide_s": glide, "changes": [["key": key, "value": value]],
        ])
        return audio.applyPatch(data)
    }

    private func state(_ audio: AudioEngine) -> [String: Any] {
        try! JSONSerialization.jsonObject(with: audio.stateJSON()) as! [String: Any]
    }

    private func control(_ audio: AudioEngine, _ key: String) -> [String: Any] {
        (state(audio)["controls"] as! [[String: Any]]).first { $0["key"] as? String == key }!
    }

    func testPatchUndoRestoresOriginalValueAndFollower() throws {
        let audio = AudioEngine()
        let result = patch(audio, "master.volume", 0.2)
        let undo = try JSONSerialization.data(withJSONObject: result["undo"]!)
        audio.applyPatch(undo)
        XCTAssertEqual(control(audio, "master.volume")["value"] as! Double, 0.7, accuracy: 0.001)

        let child = patch(audio, "water.depth", 0.3)
        audio.applyPatch(try JSONSerialization.data(withJSONObject: child["undo"]!))
        XCTAssertEqual(control(audio, "water.depth")["overridden"] as? Bool, false)
    }

    func testSnapshotPreservesLatestEditsBeyondOldHistoryLimit() {
        let source = AudioEngine()
        var controls: [Control] = []
        source.readControls(&controls)
        let volumeID = controls.first { $0.key == "master.volume" }!.id
        source.setControl(volumeID, 0.9)
        _ = patch(source, "master.volume", 0.3)
        _ = patch(source, "water.depth", 0.4)
        _ = patch(source, "flight.speed", 18)
        let gainID = controls.first { $0.key == "water.gain" }!.id
        source.setControl(gainID, 1.5)
        source.releaseControl(gainID)
        for i in 0..<70 { _ = patch(source, "wind.speed", Double(i % 10)) }
        let restored = AudioEngine()
        restored.restoreState(source.stateJSON())
        XCTAssertEqual(control(restored, "master.volume")["value"] as! Double, 0.3, accuracy: 0.001)
        XCTAssertEqual(control(restored, "water.depth")["value"] as! Double, 0.4, accuracy: 0.001)
        XCTAssertEqual(control(restored, "water.depth")["overridden"] as? Bool, true)
        XCTAssertEqual(control(restored, "water.speed")["overridden"] as? Bool, false)
        XCTAssertEqual(control(restored, "flight.speed")["value"] as! Double, 18, accuracy: 0.001)
        XCTAssertEqual(control(restored, "water.gain")["value"] as! Double, 1.5, accuracy: 0.001)
        XCTAssertEqual(control(restored, "water.gain")["overridden"] as? Bool, false)
    }

    func testFlightSwitchHasThreePlaces() {
        let audio = AudioEngine()
        XCTAssertEqual(audio.place, .home)
        audio.toggleFlight()
        XCTAssertEqual(audio.place, .flying)
        audio.toggleFlight()
        XCTAssertEqual(audio.place, .standing)
        XCTAssertEqual(audio.flying, true)
        audio.toggleFlight()
        XCTAssertEqual(audio.place, .flying)
        // The prompt layer: "стоп" stands still in the world, "домой" goes home.
        _ = patch(audio, "flight.on", "стоп")
        XCTAssertEqual(audio.place, .standing)
        let sw = (state(audio)["switches"] as! [[String: Any]]).first { $0["key"] as? String == "flight.on" }!
        XCTAssertEqual(sw["value"] as? String, "stand")
        _ = patch(audio, "flight.on", "домой")
        XCTAssertEqual(audio.place, .home)
        XCTAssertEqual(audio.flying, false)
        audio.setMoves(forward: 1, strafe: 0, turn: 0, run: false)
        audio.setMoves(forward: 0, strafe: 0, turn: 0, run: false)
    }

    func testSnapshotPreservesGlideTargetAndPendingObjectPosition() {
        let source = AudioEngine()
        let tree = (state(source)["objects"] as! [[String: Any]]).first {
            $0["kind"] as? String == "tree" && $0["movable"] as? Bool == true
        }!
        let id = (tree["id"] as! NSNumber).uint32Value
        source.setObjectPosition(id: id, SIMD3(55, -8, -1.7))
        _ = patch(source, "master.volume", 0.25, glide: 120)
        let restored = AudioEngine()
        restored.restoreState(source.stateJSON())
        XCTAssertEqual(control(restored, "master.volume")["value"] as! Double, 0.25, accuracy: 0.001)
        let object = (state(restored)["objects"] as! [[String: Any]]).first {
            ($0["id"] as! NSNumber).uint32Value == id
        }!
        XCTAssertEqual((object["pos"] as! [Double])[0], 55, accuracy: 0.001)
    }
}
