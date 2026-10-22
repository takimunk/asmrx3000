import AppKit
import Combine
import SwiftUI

/// The menu bar item and the panel it opens.
///
/// This used to be SwiftUI's `MenuBarExtra(.window)`. On macOS 27 its window stopped showing
/// Metal views: the scene and the lever tracks rendered every frame but stayed black on
/// screen (the same content in an ordinary panel shows fine). It also had no API to open the
/// panel from code, which the end of onboarding needs. So the app keeps its own status item
/// and a borderless panel under it, like the old window: it does not activate the app, takes
/// the keyboard, and closes when the user clicks elsewhere.
@MainActor
final class MenuBarController: NSObject, NSWindowDelegate {
    static private(set) var shared: MenuBarController?

    let model: AppModel
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private var panel: MenuPanel?
    private var bag = Set<AnyCancellable>()
    private var outsideClicks: Any?
    /// When the panel last closed because focus left it: a click on the icon that caused it
    /// must not reopen the panel at once.
    private var closedAt = Date.distantPast

    init(model: AppModel) {
        self.model = model
        super.init()
        Self.shared = self
        if let button = item.button {
            button.image = model.icon.image
            button.target = self
            button.action = #selector(toggle)
            button.sendAction(on: [.leftMouseDown, .rightMouseDown])
            button.setAccessibilityLabel("ASMR X3000")
        }
        model.icon.$image
            .sink { [weak self] image in self?.item.button?.image = image }
            .store(in: &bag)
    }

    var isOpen: Bool { panel?.isVisible == true }

    @objc private func toggle() {
        if isOpen {
            close()
        } else if Date().timeIntervalSince(closedAt) > 0.25 {
            open()
        }
    }

    func open() {
        let panel = self.panel ?? makePanel()
        self.panel = panel
        place(panel)
        panel.makeKeyAndOrderFront(nil)
        item.button?.highlight(true)
        if outsideClicks == nil {
            outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
                MainActor.assumeIsolated { self?.close() }
            }
        }
    }

    func close() {
        guard let panel, panel.isVisible else { return }
        panel.orderOut(nil)
        closedAt = Date()
        item.button?.highlight(false)
        if let outsideClicks { NSEvent.removeMonitor(outsideClicks) }
        outsideClicks = nil
    }

    private func makePanel() -> MenuPanel {
        let host = NSHostingController(rootView: Popover(model: model))
        host.sizingOptions = [.preferredContentSize]
        let panel = MenuPanel(contentRect: NSRect(x: 0, y: 0, width: Popover.width, height: 600),
                              styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.contentViewController = host
        panel.level = .popUpMenu
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = false
        panel.delegate = self
        // Rounded like the system's menu bar windows.
        host.view.wantsLayer = true
        host.view.layer?.cornerRadius = 12
        host.view.layer?.cornerCurve = .continuous
        host.view.layer?.masksToBounds = true
        host.view.layer?.backgroundColor = NSColor(Ink.panel).cgColor
        return panel
    }

    /// Under the icon, its top 6 pt below the menu bar, kept on the icon's screen.
    private func place(_ panel: NSPanel) {
        let size = panel.frame.size
        guard let button = item.button, let window = button.window else {
            panel.center()
            return
        }
        let icon = window.convertToScreen(button.convert(button.bounds, to: nil))
        let screen = window.screen ?? NSScreen.main
        let visible = screen?.visibleFrame ?? icon
        let x = min(max(icon.midX - size.width / 2, visible.minX + 8), visible.maxX - size.width - 8)
        panel.setFrameTopLeftPoint(NSPoint(x: x, y: icon.minY - 6))
    }

    // MARK: NSWindowDelegate

    /// The content grows and shrinks (levers expand): keep the top edge under the icon.
    func windowDidResize(_ notification: Notification) {
        guard let panel, panel.isVisible else { return }
        place(panel)
    }

    /// Focus went elsewhere (another app, the onboarding window): close, like a menu.
    func windowDidResignKey(_ notification: Notification) {
        close()
    }
}

/// A borderless panel that can still take the keyboard (prompt field, shortcuts).
final class MenuPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// Esc closes it, as it closed the menu bar window.
    override func cancelOperation(_ sender: Any?) {
        MenuBarController.shared?.close()
    }
}
