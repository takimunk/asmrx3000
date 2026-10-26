import AVFoundation
import CoreAudio
import Foundation

// Voice input for the prompt: record the microphone, send it to fal.ai Whisper, put the words
// into the prompt field. See docs/prompt-control.md ("Голос").

enum VoiceError: LocalizedError {
    case noKey
    case denied
    case noMicrophone
    case recording(String)
    case tooShort
    case nothingHeard
    case offline
    case timeout
    case http(Int, String)
    case badReply

    var errorDescription: String? {
        switch self {
        case .noKey: return tr("No fal.ai key: set FAL_KEY or add the key to the keychain (account fal).")
        case .denied: return tr("No microphone access: System Settings → Privacy & Security → Microphone.")
        case .noMicrophone: return tr("No microphone found.")
        case let .recording(why): return tr("Could not record: %@.", why)
        case .tooShort: return tr("Too short: hold the button and speak.")
        case .nothingHeard: return tr("Heard nothing. Try again.")
        case .offline: return tr("No network: speech can not be recognised.")
        case .timeout: return tr("fal.ai did not answer in time.")
        case let .http(code, body):
            let detail = PromptError.detail(body)
            switch code {
            case 401, 403: return tr("fal.ai key rejected (%d).", code)
            case 429: return tr("fal.ai: rate limit or out of credit (429).")
            case 400, 422: return tr("fal.ai rejected the recording (%d)", code) + (detail.isEmpty ? "." : ": \(detail)")
            default: return tr("fal.ai error (%d)", code) + (detail.isEmpty ? "." : ": \(detail)")
            }
        case .badReply: return tr("fal.ai replied with something unreadable.")
        }
    }

    static func from(_ error: Error) -> VoiceError {
        if let e = error as? VoiceError { return e }
        if let p = error as? PromptError, case .timeout = p { return .timeout }
        if let u = error as? URLError {
            switch u.code {
            case .timedOut: return .timeout
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed: return .offline
            default: break
            }
        }
        return .recording(error.localizedDescription)
    }
}

/// Records the microphone into a 16 kHz mono 16-bit WAV (small, and every decoder reads it).
/// Prefers the built-in microphone: recording from AirPods would switch them to the headset
/// profile and spoil playback while you speak.
final class VoiceRecorder: NSObject, AVCaptureFileOutputRecordingDelegate, @unchecked Sendable {
    /// Longest recording; it stops by itself and is sent.
    static let maxSeconds: Double = 30
    /// Called (any thread) when the recording stopped by itself at `maxSeconds`.
    var onLimit: (() -> Void)?

    private let session = AVCaptureSession()
    private let output = AVCaptureAudioFileOutput()
    private let lock = NSLock()
    private var waiting: CheckedContinuation<URL, Error>?
    private var finished: Result<URL, Error>?
    private var started: Date?

    static func permission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
        default: return false
        }
    }

    static func device() -> AVCaptureDevice? {
        let types: [AVCaptureDevice.DeviceType]
        if #available(macOS 14, *) {
            types = [.microphone, .external]
        } else {
            types = [.builtInMicrophone, .externalUnknown]
        }
        let all = AVCaptureDevice.DiscoverySession(deviceTypes: types, mediaType: .audio, position: .unspecified).devices
        let builtIn = Int32(bitPattern: kAudioDeviceTransportTypeBuiltIn)
        return all.first { $0.transportType == builtIn } ?? AVCaptureDevice.default(for: .audio) ?? all.first
    }

    /// Blocks while the session starts: call off the main thread.
    func start() throws {
        guard let device = Self.device() else { throw VoiceError.noMicrophone }
        let input = try AVCaptureDeviceInput(device: device)
        session.beginConfiguration()
        for i in session.inputs { session.removeInput(i) }
        guard session.canAddInput(input) else {
            session.commitConfiguration()
            throw VoiceError.recording(tr("the microphone is busy"))
        }
        session.addInput(input)
        if !session.outputs.contains(output) {
            guard session.canAddOutput(output) else {
                session.commitConfiguration()
                throw VoiceError.recording(tr("the file can not be recorded"))
            }
            session.addOutput(output)
        }
        output.audioSettings = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
        ]
        output.maxRecordedDuration = CMTime(seconds: Self.maxSeconds, preferredTimescale: 600)
        session.commitConfiguration()
        session.startRunning()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("asmr-voice-\(UUID().uuidString).wav")
        lock.lock()
        finished = nil
        started = Date()
        lock.unlock()
        output.startRecording(to: url, outputFileType: .wav, recordingDelegate: self)
    }

    /// Stops and returns the file (or the one that already stopped at the limit).
    func stop() async throws -> URL {
        // A very quick click: the output may not have begun yet.
        for _ in 0..<20 where !output.isRecording && peekFinished() == nil {
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        let short = Date().timeIntervalSince(started ?? .distantPast) < 0.4
        let url: URL = try await withCheckedThrowingContinuation { c in
            lock.lock()
            if let done = finished {
                lock.unlock()
                c.resume(with: done)
                return
            }
            waiting = c
            lock.unlock()
            if output.isRecording { output.stopRecording() } else { finish(.failure(VoiceError.tooShort)) }
        }
        if short {
            try? FileManager.default.removeItem(at: url)
            throw VoiceError.tooShort
        }
        return url
    }

    private func peekFinished() -> Result<URL, Error>? {
        lock.lock()
        defer { lock.unlock() }
        return finished
    }

    private func finish(_ result: Result<URL, Error>) {
        lock.lock()
        finished = result
        let c = waiting
        waiting = nil
        lock.unlock()
        c?.resume(with: result)
        if c == nil, case .success = result { onLimit?() }
    }

    func fileOutput(_ output: AVCaptureFileOutput, didFinishRecordingTo url: URL, from connections: [AVCaptureConnection], error: Error?) {
        session.stopRunning()
        // Hitting the duration limit reports an "error" that still finished the file.
        if let error = error as NSError?, (error.userInfo[AVErrorRecordingSuccessfullyFinishedKey] as? Bool) != true {
            finish(.failure(VoiceError.recording(error.localizedDescription)))
        } else {
            finish(.success(url))
        }
    }
}

/// fal.ai Whisper (model `fal-ai/whisper`), synchronous endpoint: the audio goes inline as a
/// base64 data URI, the reply is `{"text": …, "chunks": […]}`. Key: FAL_KEY, else the keychain
/// (service com.asmrx3000.prompt, account fal).
struct FalWhisper {
    let key: String
    static let endpoint = URL(string: "https://fal.run/fal-ai/whisper")!

    /// Cached by Keychain (read once per run, replaced when a key is saved in the app).
    static var storedKey: String? { Keychain.stored(.fal) }

    func transcribe(_ file: URL, language: String = L10n.current.rawValue) async throws -> String {
        let audio = try Data(contentsOf: file)
        let body: [String: Any] = [
            "audio_url": "data:audio/wav;base64," + audio.base64EncodedString(),
            "task": "transcribe",
            "language": language,
            "chunk_level": "none",
        ]
        var req = URLRequest(url: Self.endpoint, timeoutInterval: 30)
        req.httpMethod = "POST"
        req.setValue("Key \(key)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, response) = try await URLSession.shared.data(for: req)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard code == 200 else { throw VoiceError.http(code, String(decoding: data.prefix(400), as: UTF8.self)) }
        guard let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any], let text = o["text"] as? String else {
            throw VoiceError.badReply
        }
        let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !words.isEmpty else { throw VoiceError.nothingHeard }
        return words
    }
}

/// `ASMRX3000 --voice-check [file.wav]`: transcribes a file (default: a generated tone, which
/// should come back empty or as noise) through fal.ai, if FAL_KEY is set. Never prints the key.
enum VoiceCheck {
    static func run(_ path: String?) {
        guard let key = FalWhisper.storedKey else {
            print("FAL_KEY is not set (and no keychain item com.asmrx3000.prompt / fal): skipped")
            return
        }
        let url = path.map { URL(fileURLWithPath: $0) } ?? tone()
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            let start = Date()
            do {
                let text = try await FalWhisper(key: key).transcribe(url)
                print("fal-ai/whisper \(Int(Date().timeIntervalSince(start) * 1000)) ms: \(text)")
            } catch {
                print("fal-ai/whisper failed: \(VoiceError.from(error).errorDescription ?? "") (\(error))")
            }
            done.signal()
        }
        done.wait()
    }

    /// One second of a 440 Hz tone as 16 kHz mono WAV.
    static func tone() -> URL {
        let rate = 16_000, n = rate
        var d = Data()
        func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
        func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { d.append(contentsOf: $0) } }
        d.append(contentsOf: Array("RIFF".utf8)); u32(UInt32(36 + n * 2)); d.append(contentsOf: Array("WAVEfmt ".utf8))
        u32(16); u16(1); u16(1); u32(UInt32(rate)); u32(UInt32(rate * 2)); u16(2); u16(16)
        d.append(contentsOf: Array("data".utf8)); u32(UInt32(n * 2))
        for i in 0..<n { u16(UInt16(bitPattern: Int16(sin(Double(i) * 2 * .pi * 440 / Double(rate)) * 8000))) }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("asmr-voice-check.wav")
        try? d.write(to: url)
        return url
    }
}
