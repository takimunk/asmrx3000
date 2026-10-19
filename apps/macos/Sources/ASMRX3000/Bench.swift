import AppKit
import CAsmr
import SwiftUI

/// `ASMRX3000 --bench`: shows the popover in an ordinary window for 20 s with audio playing,
/// then hides it for 10 s, printing the process CPU for each phase. Measures what the UI costs
/// without having to click the menu bar item. The audio engine's own share is reported too.
/// With `ASMR_PROFILE=1` each phase also prints the engine's per-section profile (profile.rs).
enum Bench {
    private static var window: NSWindow?
    private static var model: AppModel?

    static func start() {
        DispatchQueue.main.async { run() }
    }

    private static func cpuSeconds() -> Double { userSeconds() + systemSeconds() }

    private static func userSeconds() -> Double {
        var u = rusage()
        getrusage(RUSAGE_SELF, &u)
        return Double(u.ru_utime.tv_sec) + Double(u.ru_utime.tv_usec) / 1e6
    }

    private static func systemSeconds() -> Double {
        var u = rusage()
        getrusage(RUSAGE_SELF, &u)
        return Double(u.ru_stime.tv_sec) + Double(u.ru_stime.tv_usec) / 1e6
    }

    private static func run() {
        let model = AppModel()
        self.model = model
        model.audio.play()
        let host = NSHostingView(rootView: Popover(model: model))
        let w = NSWindow(contentRect: NSRect(x: 200, y: 200, width: Popover.width, height: 700),
                         styleMask: [.titled], backing: .buffered, defer: false)
        w.contentView = host
        w.level = .statusBar
        w.setContentSize(host.fittingSize)
        if ProcessInfo.processInfo.environment["BENCH_NO_UI"] == nil { w.makeKeyAndOrderFront(nil) }
        NSApp.activate(ignoringOtherApps: true)
        window = w

        func measure(_ label: String, seconds: Double, then: @escaping () -> Void) {
            let c0 = cpuSeconds(), s0 = systemSeconds(), t0 = Date(), f0 = model.stage.frames
            let profiling = ProcessInfo.processInfo.environment["ASMR_PROFILE"] != nil
            if profiling { asmr_profile_enable() }
            DispatchQueue.main.asyncAfter(deadline: .now() + seconds) {
                let cpu = (cpuSeconds() - c0) / Date().timeIntervalSince(t0) * 100
                let dsp = Double(model.stage.telemetry.dspLoad) * 100
                let fps = Double(model.stage.frames - f0) / Date().timeIntervalSince(t0)
                let sys = (systemSeconds() - s0) / Date().timeIntervalSince(t0) * 100
                print(String(format: "%@: process %.1f %% of one core (system %.1f %%), %.0f fps (audio engine ≈ %.1f %%)", label, cpu, sys, fps, dsp))
                if profiling, let text = asmr_profile_text_alloc(label) {
                    print(String(cString: text))
                    asmr_string_free(text)
                }
                then()
            }
        }
        // Let shaders compile and caches settle first.
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
            measure("popover open", seconds: 20) {
                w.orderOut(nil)
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    measure("popover closed", seconds: 10) { exit(0) }
                }
            }
        }
    }
}
