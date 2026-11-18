import AppKit
import ApplicationServices

// Independent prototype: it does not load or modify Code Pet's settings.
func axRead(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}
func axWritable(_ element: AXUIElement, _ name: String) -> Bool {
    var writable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element, name as CFString, &writable) == .success && writable.boolValue
}

struct WindowFailure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

final class WindowTarget {
    let local: NSWindow?
    let element: AXUIElement?
    let pid: pid_t
    let name: String
    let title: String
    var original: CGRect = .zero
    var originallyMinimized = false
    var canResize = true
    init(local: NSWindow) {
        self.local = local; element = nil; pid = getpid()
        name = "内置示例"; title = "可编辑便笺"
    }
    init(element: AXUIElement, pid: pid_t, name: String, title: String) {
        self.element = element; self.pid = pid; self.name = name; self.title = title; local = nil
        AXUIElementSetMessagingTimeout(element, 0.15)
    }
    func frame(primaryTop: CGFloat) throws -> CGRect {
        if let local { return local.frame }
        guard let element,
              let position = axRead(element, kAXPositionAttribute), CFGetTypeID(position) == AXValueGetTypeID(),
              let size = axRead(element, kAXSizeAttribute), CFGetTypeID(size) == AXValueGetTypeID() else {
            throw WindowFailure(message: "窗口暂时无响应或已关闭；可以重试或解除收纳。")
        }
        var point = CGPoint.zero, dimensions = CGSize.zero
        guard AXValueGetValue(unsafeBitCast(position, to: AXValue.self), .cgPoint, &point),
              AXValueGetValue(unsafeBitCast(size, to: AXValue.self), .cgSize, &dimensions) else {
            throw WindowFailure(message: "无法读取窗口位置。")
        }
        return DrawerGeometry.appKitRect(fromAX: CGRect(origin: point, size: dimensions), primaryTop: primaryTop)
    }
    func snapshot(primaryTop: CGFloat) throws {
        original = try frame(primaryTop: primaryTop)
        if let local { originallyMinimized = local.isMiniaturized; return }
        guard let element, axWritable(element, kAXPositionAttribute) else {
            throw WindowFailure(message: "这个窗口不支持位置控制，请换一个普通窗口。")
        }
        guard axRead(element, "AXFullScreen") as? Bool != true else {
            throw WindowFailure(message: "请先退出该窗口的全屏模式。")
        }
        originallyMinimized = axRead(element, kAXMinimizedAttribute) as? Bool ?? false
        canResize = axWritable(element, kAXSizeAttribute)
    }
    func setFrame(_ frame: CGRect, primaryTop: CGFloat, resize: Bool = true) throws {
        if let local { local.setFrame(frame, display: true); return }
        guard let element else { return }
        let ax = DrawerGeometry.axRect(fromAppKit: frame, primaryTop: primaryTop)
        var point = ax.origin, size = ax.size
        guard let p = AXValueCreate(.cgPoint, &point), let s = AXValueCreate(.cgSize, &size) else { return }
        let move = AXUIElementSetAttributeValue(element, kAXPositionAttribute as CFString, p)
        guard move == .success else { throw WindowFailure(message: "移动窗口失败（\(move.rawValue)）。") }
        if resize && canResize {
            // Some apps clamp sizes; the caller reads back the actual result.
            _ = AXUIElementSetAttributeValue(element, kAXSizeAttribute as CFString, s)
            _ = AXUIElementSetAttributeValue(element, kAXPositionAttribute as CFString, p)
        }
    }
    func setMinimizedState(_ value: Bool) throws {
        if let local {
            if value && !local.isMiniaturized { local.miniaturize(nil) }
            else if !value && local.isMiniaturized { local.deminiaturize(nil) }
            return
        }
        guard let element else { return }
        if axRead(element, kAXMinimizedAttribute) as? Bool == value { return }
        let error = AXUIElementSetAttributeValue(element, kAXMinimizedAttribute as CFString,
                                                value ? kCFBooleanTrue : kCFBooleanFalse)
        guard error == .success else { throw WindowFailure(message: "无法恢复窗口原有状态（\(error.rawValue)），请检查辅助功能权限。") }
    }
    func conceal(primaryTop: CGFloat, displays: [CGRect]) throws {
        if let local {
            local.orderOut(nil)
            guard !local.isVisible else { throw WindowFailure(message: "窗口未能收起，请重试。") }
            return
        }
        let before = try frame(primaryTop: primaryTop)
        for candidate in DrawerGeometry.parkingCandidates(size: before.size, screens: displays) {
            do {
                try setFrame(candidate, primaryTop: primaryTop, resize: false)
                let actual = try frame(primaryTop: primaryTop)
                if DrawerGeometry.isOutsideDisplays(actual, screens: displays) { return }
            } catch { continue }
        }
        // Never substitute minimizing or app-wide hide for the requested top-bar state.
        try setFrame(before, primaryTop: primaryTop, resize: false)
        throw WindowFailure(message: "此 App 不允许窗口完全藏起，已保留原窗口；不会改用最小化。")
    }
    func present(in frame: CGRect, primaryTop: CGFloat) throws {
        try setMinimizedState(false) // Only undo an existing minimized state; collapse never creates one.
        try setFrame(frame, primaryTop: primaryTop)
        raise()
    }
    func raise() {
        if let local { local.orderFrontRegardless() }
        else if let element { AXUIElementPerformAction(element, kAXRaiseAction as CFString) }
    }
    func restore(primaryTop: CGFloat) throws {
        if let element {
            var role: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &role) == .invalidUIElement { return }
            if NSRunningApplication(processIdentifier: pid)?.isTerminated != false { return }
        }
        // Restore geometry while still minimized when supported, then restore original visibility.
        do { try setFrame(original, primaryTop: primaryTop) }
        catch {
            // Some apps refuse geometry writes while minimized. Always attempt visibility recovery.
            try setMinimizedState(false)
            try setFrame(original, primaryTop: primaryTop)
        }
        try setMinimizedState(originallyMinimized)
        if let local, !originallyMinimized { local.orderFrontRegardless() }
    }
}

struct RecoveryRecord: Codable {
    var pid: Int32
    var launchedAt: Double
    var title: String
    var x: Double, y: Double, width: Double, height: Double
    var minimized: Bool
}

final class HeaderView: NSView {
    weak var owner: DrawerDemo?
    var aperture: CGRect? { didSet { needsDisplay = true } }
    private var tracking: NSTrackingArea?
    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.clear(bounds)
        NSColor.black.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 18, yRadius: 18).fill()
        // The top is attached to the screen. Only the lower outer corners are rounded.
        NSBezierPath(rect: CGRect(x: 0, y: max(0, bounds.height - 18), width: bounds.width, height: 18)).fill()
        if let aperture {
            context.saveGState()
            context.addPath(CGPath(roundedRect: aperture, cornerWidth: 10, cornerHeight: 10, transform: nil))
            context.clip()
            context.clear(aperture)
            context.restoreGState()
        }
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        tracking = NSTrackingArea(rect: bounds, options: [.activeAlways, .mouseEnteredAndExited, .inVisibleRect], owner: self)
        addTrackingArea(tracking!)
    }
    override func mouseEntered(with event: NSEvent) { owner?.poll() }
    override func mouseDown(with event: NSEvent) {
        guard owner?.headerRect.contains(NSEvent.mouseLocation) == true else { return }
        owner?.startDrag(at: NSEvent.mouseLocation)
    }
    override func mouseDragged(with event: NSEvent) { owner?.drag(to: NSEvent.mouseLocation) }
    override func mouseUp(with event: NSEvent) { owner?.endDrag() }
}
final class TopPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

final class DrawerDemo: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let queue = DispatchQueue(label: "com.codepet.window-drawer-demo.ax", qos: .userInitiated)
    var panel: TopPanel!
    var header: HeaderView!
    var headline: NSTextField!
    var detail: NSTextField!
    var avatarLabel: NSTextField!
    var headerButtons: [NSButton] = []
    var pinButton: NSButton!
    var chooseButton: NSButton!
    var releaseButton: NSButton!
    var chooser: NSWindow?
    var chooserStack: NSStackView?
    var chooserStatus: NSTextField?
    var chooserRefresh: NSButton?
    var sample: NSWindow?
    var target: WindowTarget?
    var candidates: [WindowTarget] = []
    var currentFrame = CGRect.zero
    var policy = DrawerPolicy()
    var timer: Timer?
    var isOpen = false, pinned = false, busy = false
    var dragging = false
    var dragStart = CGPoint.zero, headerStart = CGRect.zero
    var pendingMove: CGRect?
    var moving = false
    var serial = 0
    var lastRead = 0.0
    var statusItem: NSStatusItem!
    var sampleEdits = 0
    var sampleEditLabel: NSTextField?
    var recoveryKey = "windowDrawerRecoveryV1"
    var primaryTop: CGFloat { NSScreen.screens.first?.frame.maxY ?? 0 }
    var headerHeight: CGFloat { max(30, screen.safeAreaInsets.top) + 46 }
    var headerRect: CGRect {
        CGRect(x: panel.frame.minX, y: panel.frame.maxY - headerHeight,
               width: panel.frame.width, height: headerHeight)
    }
    var screen: NSScreen {
        NSScreen.screens.first(where: { $0.frame.contains(panel?.frame.center ?? NSEvent.mouseLocation) }) ?? NSScreen.screens[0]
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        makeMenu()
        let display = NSScreen.screens.first(where: { $0.frame.contains(NSEvent.mouseLocation) }) ?? NSScreen.screens[0]
        // Offset from the production pet so both can run together.
        let safe = max(30, display.safeAreaInsets.top)
        panel = TopPanel(contentRect: CGRect(x: display.frame.minX + display.frame.width * 0.24 - 260,
                                            y: display.frame.maxY - safe - 46, width: 520, height: safe + 46),
                         styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.title = "Code Pet 窗口抽屉 Demo"
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.level = .floating
        panel.isOpaque = false; panel.backgroundColor = .clear
        panel.hasShadow = false; panel.hidesOnDeactivate = false
        panel.acceptsMouseMovedEvents = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        header = HeaderView(frame: CGRect(origin: .zero, size: panel.frame.size))
        header.owner = self; header.wantsLayer = true
        header.layer?.backgroundColor = NSColor.clear.cgColor
        panel.contentView = header
        let pet = label("🐱", size: 23, color: .white)
        avatarLabel = pet
        pet.frame = CGRect(x: 12, y: 8, width: 34, height: 32); header.addSubview(pet)
        headline = label("窗口抽屉 · Demo", size: 12, color: .white)
        headline.font = .systemFont(ofSize: 12, weight: .semibold)
        headline.frame = CGRect(x: 51, y: 24, width: 230, height: 16); header.addSubview(headline)
        detail = label("选择窗口，或先体验示例", size: 10, color: .lightGray)
        detail.frame = CGRect(x: 51, y: 7, width: 230, height: 15); header.addSubview(detail)
        chooseButton = button("选择", action: #selector(showChooser))
        pinButton = button("固定", action: #selector(togglePin))
        releaseButton = button("释放", action: #selector(releaseSelected))
        let exitButton = button("退出", action: #selector(quit))
        headerButtons = [chooseButton!, pinButton!, releaseButton!, exitButton]
        for (index, button) in headerButtons.enumerated() {
            button.appearance = NSAppearance(named: .darkAqua)
            button.attributedTitle = NSAttributedString(string: button.title,
                attributes: [.foregroundColor: NSColor.white, .font: NSFont.systemFont(ofSize: 11)])
            button.frame = CGRect(x: 284 + index * 57, y: 10, width: 54, height: 27)
            header.addSubview(button)
        }
        refreshHeader()
        let bounded = DrawerGeometry.clamp(panel.frame, to: display.frame)
        panel.setFrame(bounded, display: true)
        panel.orderFrontRegardless()
        NotificationCenter.default.addObserver(self, selector: #selector(screensChanged), name: NSApplication.didChangeScreenParametersNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(spaceChanged), name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(appActivated(_:)), name: NSWorkspace.didActivateApplicationNotification, object: nil)
        timer = Timer.scheduledTimer(withTimeInterval: 0.025, repeats: true) { [weak self] _ in self?.poll() }
        RunLoop.main.add(timer!, forMode: .common)
        if CommandLine.arguments.contains("--smoke") { runSmoke() }
        else { showChooser() }
    }

    func makeMenu() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "🐱▾"
        let menu = NSMenu()
        for (title, selector) in [("显示顶部入口", #selector(showHeader)), ("收起到顶部", #selector(collapseNow)), ("选择窗口…", #selector(showChooser)), ("体验示例窗口", #selector(useSample)),
                                   ("恢复上次收纳窗口", #selector(recover)), ("释放当前窗口", #selector(releaseSelected)),
                                   ("退出 Demo", #selector(quit))] {
            let item = NSMenuItem(title: title, action: selector, keyEquivalent: "")
            if selector == #selector(quit) { item.keyEquivalent = "q" }
            item.target = self; menu.addItem(item)
        }
        statusItem.menu = menu
        let appMenu = NSMenu()
        let root = NSMenuItem(); root.submenu = menu.copy() as? NSMenu
        appMenu.addItem(root); NSApp.mainMenu = appMenu
    }
    func label(_ text: String, size: CGFloat, color: NSColor = .labelColor) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.font = .systemFont(ofSize: size); field.textColor = color
        field.lineBreakMode = .byTruncatingTail
        return field
    }
    func button(_ title: String, action: Selector) -> NSButton {
        let control = NSButton(title: title, target: self, action: action)
        control.bezelStyle = .rounded; control.font = .systemFont(ofSize: 11)
        return control
    }
    func refreshHeader(_ message: String? = nil) {
        headline.stringValue = target.map { "\($0.name) · \($0.title)" } ?? "窗口抽屉 · Demo"
        detail.stringValue = message ?? (target == nil ? "选择窗口，或先体验示例" : pinned ? "已固定 · 点击固定恢复自动收起" : isOpen ? "移开 0.65 秒收起 · 顶栏可左右拖动" : "悬停展开 · 已收纳到顶部")
        pinButton.title = pinned ? "已固定" : "固定"
        pinButton.attributedTitle = NSAttributedString(string: pinButton.title,
            attributes: [.foregroundColor: NSColor.white, .font: NSFont.systemFont(ofSize: 11)])
        pinButton.toolTip = pinned ? "取消固定，恢复鼠标移开自动收起" : "固定展开，适合输入及菜单操作"
        pinButton.isEnabled = target != nil && !busy
        releaseButton.isEnabled = target != nil && !busy
        chooseButton.isEnabled = !busy
        layoutHeaderControls()
    }

    func layoutHeaderControls() {
        let offset = panel.frame.height - headerHeight
        avatarLabel.frame = CGRect(x: 12, y: offset + 8, width: 34, height: 32)
        let textWidth = max(160, panel.frame.width - 290)
        headline.frame = CGRect(x: 51, y: offset + 24, width: textWidth, height: 16)
        detail.frame = CGRect(x: 51, y: offset + 7, width: textWidth, height: 15)
        for (index, button) in headerButtons.enumerated() {
            button.frame = CGRect(x: panel.frame.width - 236 + CGFloat(index) * 57,
                                  y: offset + 10, width: 54, height: 27)
        }
    }

    func configureChrome(around window: CGRect?) {
        let display = screen.frame
        let topHeight = headerHeight
        let frame: CGRect
        if let window {
            let width = min(display.width, max(DrawerGeometry.collapsedWidth, window.width + 2 * DrawerGeometry.padding))
            let x = min(max(window.minX - DrawerGeometry.padding, display.minX), display.maxX - width)
            frame = CGRect(x: x, y: window.minY - DrawerGeometry.padding, width: width,
                           height: display.maxY - window.minY + DrawerGeometry.padding)
        } else {
            let width = min(DrawerGeometry.collapsedWidth, display.width)
            let x = min(max(panel.frame.midX - width / 2, display.minX), display.maxX - width)
            frame = CGRect(x: x, y: display.maxY - topHeight, width: width, height: topHeight)
        }
        panel.setFrame(frame, display: true)
        header.frame = CGRect(origin: .zero, size: frame.size)
        header.aperture = window.map { DrawerGeometry.localAperture(window: $0, shell: frame) }
        layoutHeaderControls()
        updateInputPassthrough()
    }

    func updateInputPassthrough() {
        let pass = !dragging && DrawerGeometry.receivesNativeInput(pointer: NSEvent.mouseLocation,
                                                                 window: currentFrame, open: isOpen)
        if panel.ignoresMouseEvents != pass { panel.ignoresMouseEvents = pass }
    }

    @objc func showChooser() {
        guard !busy else { return }
        if isOpen { pinned = false; tuck() }
        policy.reset()
        if let chooser { chooser.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true); refreshCandidates(); return }
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 600, height: 480),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "选择收纳窗口 · Code Pet Demo"; window.isReleasedWhenClosed = false; window.delegate = self
        let content = window.contentView!
        let heading = label("把一个窗口放进顶部抽屉", size: 23)
        heading.font = .systemFont(ofSize: 23, weight: .semibold); heading.frame = CGRect(x: 26, y: 414, width: 550, height: 36)
        content.addSubview(heading)
        let hint = label("悬停展开，移开收起。输入或使用菜单时，可点顶栏「固定」。", size: 12, color: .secondaryLabelColor)
        hint.frame = CGRect(x: 26, y: 387, width: 550, height: 22); content.addSubview(hint)
        let sampleButton = button("先体验可编辑示例", action: #selector(useSample))
        sampleButton.frame = CGRect(x: 26, y: 340, width: 178, height: 33); content.addSubview(sampleButton)
        let permission = button("开启辅助功能权限…", action: #selector(requestPermission))
        permission.frame = CGRect(x: 214, y: 340, width: 178, height: 33); content.addSubview(permission)
        let refresh = button("刷新窗口", action: #selector(refreshCandidates))
        refresh.frame = CGRect(x: 402, y: 340, width: 160, height: 33); content.addSubview(refresh); chooserRefresh = refresh
        chooserStatus = label("", size: 11, color: .secondaryLabelColor)
        chooserStatus!.frame = CGRect(x: 28, y: 301, width: 542, height: 31); content.addSubview(chooserStatus!)
        let scroll = NSScrollView(frame: CGRect(x: 26, y: 53, width: 548, height: 240))
        scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
        let stack = NSStackView(); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 5
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = stack
        NSLayoutConstraint.activate([stack.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor)])
        content.addSubview(scroll); chooserStack = stack
        let recovery = button("恢复上次收纳窗口", action: #selector(recover))
        recovery.frame = CGRect(x: 26, y: 11, width: 185, height: 31); content.addSubview(recovery)
        let note = label("收起只留顶部条，不进入 Dock；部分 App 暂不支持。", size: 10, color: .secondaryLabelColor)
        note.frame = CGRect(x: 222, y: 17, width: 352, height: 20); content.addSubview(note)
        window.center(); chooser = window; window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true); refreshCandidates()
    }
    func windowWillClose(_ notification: Notification) { policy.reset(requireExit: true) }
    @objc func requestPermission() {
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        _ = AXIsProcessTrustedWithOptions(options)
        NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)
        chooserStatus?.stringValue = "在辅助功能列表开启 Code Pet Drawer，再回来点击「刷新窗口」。"
    }
    @objc func refreshCandidates() {
        guard AXIsProcessTrusted() else {
            chooserStatus?.stringValue = "尚未授权：可先体验内置示例；操作其他 App 需要辅助功能权限。"
            return
        }
        chooserRefresh?.isEnabled = false
        chooserStatus?.stringValue = "正在读取普通窗口…"
        let apps = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular && $0.processIdentifier != getpid() && !$0.isHidden }
        queue.async {
            var found: [WindowTarget] = []
            for app in apps {
                let element = AXUIElementCreateApplication(app.processIdentifier)
                AXUIElementSetMessagingTimeout(element, 0.12)
                guard let windows = axRead(element, kAXWindowsAttribute) as? [AXUIElement] else { continue }
                for window in windows.prefix(30) {
                    AXUIElementSetMessagingTimeout(window, 0.12)
                    guard axRead(window, kAXSubroleAttribute) as? String == kAXStandardWindowSubrole,
                          axRead(window, "AXFullScreen") as? Bool != true else { continue }
                    found.append(WindowTarget(element: window, pid: app.processIdentifier, name: app.localizedName ?? "App",
                                              title: axRead(window, kAXTitleAttribute) as? String ?? "未命名窗口"))
                }
            }
            DispatchQueue.main.async {
                self.candidates = found
                self.chooserRefresh?.isEnabled = true
                if let stack = self.chooserStack {
                    for view in stack.arrangedSubviews { stack.removeArrangedSubview(view); view.removeFromSuperview() }
                    for (index, candidate) in found.enumerated() {
                        let row = self.button("\(candidate.name)  —  \(candidate.title)", action: #selector(self.chooseCandidate(_:)))
                        row.tag = index; row.alignment = .left
                        row.setAccessibilityLabel("收纳 \(candidate.name) \(candidate.title)")
                        stack.addArrangedSubview(row)
                        row.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -8).isActive = true
                        row.heightAnchor.constraint(equalToConstant: 34).isActive = true
                    }
                }
                self.chooserStatus?.stringValue = found.isEmpty ? "没有找到可收纳窗口，请打开一个普通窗口后刷新。" : "已授权 · 选择下面一个窗口收纳（\(found.count) 个）"
            }
        }
    }
    @objc func chooseCandidate(_ sender: NSButton) {
        guard candidates.indices.contains(sender.tag), !busy else { return }
        bind(candidates[sender.tag])
    }
    func perform<T>(_ target: WindowTarget, work: @escaping () throws -> T, completion: @escaping (Result<T, Error>) -> Void) {
        if target.local != nil { completion(Result { try work() }) }
        else { queue.async { let result = Result { try work() }; DispatchQueue.main.async { completion(result) } } }
    }
    func bind(_ next: WindowTarget) {
        guard !busy else { return }
        if target != nil {
            releaseTarget { success in if success { self.bind(next) } }
            return
        }
        busy = true; serial += 1
        let top = primaryTop, displays = NSScreen.screens.map(\.frame)
        refreshHeader("正在收纳…")
        perform(next, work: { try next.snapshot(primaryTop: top) }) { result in
            switch result {
            case .failure(let error): self.busy = false; self.showError(error)
            case .success:
                // Persist before mutating the external window.
                if next.local == nil { self.saveRecovery(next) }
                self.perform(next, work: { try next.conceal(primaryTop: top, displays: displays) }) { result in
                    self.busy = false
                    switch result {
                    case .failure(let error):
                        self.target = next
                        self.rejectHosting(next, error: error)
                    case .success:
                        self.target = next; self.currentFrame = next.original
                        self.isOpen = false; self.pinned = false
                        self.chooser?.orderOut(nil); self.policy.reset(requireExit: true)
                        self.refreshHeader()
                    }
                }
            }
        }
    }
    func saveRecovery(_ target: WindowTarget) {
        guard let launched = NSRunningApplication(processIdentifier: target.pid)?.launchDate else { return }
        let r = target.original
        let record = RecoveryRecord(pid: target.pid, launchedAt: launched.timeIntervalSince1970, title: target.title,
                                    x: r.minX, y: r.minY, width: r.width, height: r.height, minimized: target.originallyMinimized)
        if let data = try? JSONEncoder().encode(record) { UserDefaults.standard.set(data, forKey: recoveryKey) }
    }
    func desiredFrame(for target: WindowTarget) -> CGRect {
        let work = screen.visibleFrame.insetBy(dx: 12, dy: 12)
        let maxHeight = headerRect.minY - screen.visibleFrame.minY - DrawerGeometry.bodyGap - DrawerGeometry.padding
        let size = target.canResize ? CGSize(width: min(target.original.width, min(1040, work.width - 2 * DrawerGeometry.padding)),
                                            height: min(target.original.height, max(180, maxHeight))) : target.original.size
        return DrawerGeometry.hostedLayout(anchor: headerRect, windowSize: size, screen: screen.frame,
                                           headerHeight: headerHeight).window
    }
    func reveal() {
        guard let target, !busy, !isOpen else { return }
        busy = true; let top = primaryTop, desired = desiredFrame(for: target)
        refreshHeader("正在展开…")
        perform(target, work: {
            try target.present(in: desired, primaryTop: top)
            return try target.frame(primaryTop: top)
        }) { result in
            self.busy = false
            switch result {
            case .success(let actual):
                guard actual.width + 2 * DrawerGeometry.padding <= self.screen.frame.width + 1,
                      actual.minY - DrawerGeometry.padding >= self.screen.visibleFrame.minY - 1,
                      actual.maxY <= self.headerRect.minY - DrawerGeometry.bodyGap + 1 else {
                    self.rejectHosting(target, error: WindowFailure(message: "窗口的最小尺寸超出正文区域，已尝试恢复原位置。请换一个窗口。"))
                    return
                }
                self.currentFrame = actual; self.isOpen = true
                self.configureChrome(around: actual)
                self.policy.reset(); self.refreshHeader()
            case .failure(let error):
                self.rejectHosting(target, error: error)
            }
            self.panel.orderFrontRegardless()
        }
    }
    func rejectHosting(_ target: WindowTarget, error: Error) {
        busy = true; serial += 1
        let top = primaryTop
        perform(target, work: { try target.restore(primaryTop: top) }) { result in
            self.busy = false; self.isOpen = false; self.pinned = false
            self.configureChrome(around: nil)
            if case .success = result {
                if target.local == nil { UserDefaults.standard.removeObject(forKey: self.recoveryKey) }
                self.target = nil
            }
            self.policy.reset(requireExit: true)
            self.showError(error)
        }
    }
    func tuck() {
        guard let target, !busy, isOpen, !moving else { return }
        let top = primaryTop, displays = NSScreen.screens.map(\.frame)
        busy = true; refreshHeader("正在收起…")
        perform(target, work: { try target.conceal(primaryTop: top, displays: displays) }) { result in
            self.busy = false
            switch result {
            case .success:
                self.isOpen = false; self.configureChrome(around: nil); self.refreshHeader()
            case .failure(let error): self.pinned = true; self.showError(error)
            }
            self.policy.reset(requireExit: true)
        }
    }
    @objc func collapseNow() {
        guard !busy else { return }
        pinned = false
        tuck()
    }
    @objc func togglePin() {
        guard target != nil, !busy else { return }
        pinned.toggle(); policy.reset()
        if pinned && !isOpen { reveal() }
        refreshHeader()
    }
    @objc func releaseSelected() { releaseTarget { _ in } }
    func releaseTarget(completion: @escaping (Bool) -> Void) {
        guard let target, !busy else { completion(self.target == nil); return }
        // A detached display must not turn a normal release into an offscreen window.
        if let display = NSScreen.screens.max(by: {
            $0.visibleFrame.intersection(target.original).area < $1.visibleFrame.intersection(target.original).area
        }), display.visibleFrame.intersects(target.original) {
            target.original = DrawerGeometry.clamp(target.original, to: display.visibleFrame)
        } else {
            target.original = DrawerGeometry.clamp(target.original, to: screen.visibleFrame)
        }
        busy = true; serial += 1; pendingMove = nil
        let top = primaryTop
        perform(target, work: { try target.restore(primaryTop: top) }) { result in
            self.busy = false
            switch result {
            case .success:
                if target.local == nil { UserDefaults.standard.removeObject(forKey: self.recoveryKey) }
                self.target = nil; self.isOpen = false; self.pinned = false; self.policy.reset(requireExit: true)
                self.configureChrome(around: nil)
                self.refreshHeader("已恢复原位置，可重新选择窗口"); completion(true)
            case .failure(let error): self.showError(error); completion(false)
            }
        }
    }
    @objc func recover() {
        guard !busy else { return }
        if target != nil { releaseSelected(); return }
        guard AXIsProcessTrusted(), let data = UserDefaults.standard.data(forKey: recoveryKey),
              let record = try? JSONDecoder().decode(RecoveryRecord.self, from: data) else {
            refreshHeader("没有可恢复记录，或尚未开启辅助功能权限。"); return
        }
        guard let app = NSRunningApplication(processIdentifier: record.pid),
              let launched = app.launchDate, abs(launched.timeIntervalSince1970 - record.launchedAt) < 1 else {
            refreshHeader("原 App 已退出。请重新选择窗口。"); return
        }
        busy = true
        queue.async {
            let appElement = AXUIElementCreateApplication(record.pid)
            AXUIElementSetMessagingTimeout(appElement, 0.2)
            let windows = (axRead(appElement, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter {
                AXUIElementSetMessagingTimeout($0, 0.15)
                return axRead($0, kAXTitleAttribute) as? String == record.title
            }
            guard windows.count == 1 else {
                DispatchQueue.main.async { self.busy = false; self.refreshHeader("窗口无法唯一匹配，请从 Dock 恢复或重新选择。") }
                return
            }
            let recovered = WindowTarget(element: windows[0], pid: record.pid, name: app.localizedName ?? "App", title: record.title)
            recovered.original = CGRect(x: record.x, y: record.y, width: record.width, height: record.height)
            recovered.originallyMinimized = record.minimized
            DispatchQueue.main.async {
                self.busy = false
                let alert = NSAlert()
                alert.messageText = "确认恢复这个窗口？"
                alert.informativeText = "\(recovered.name) — \(recovered.title)\n\n这是按进程和标题找到的候选。若原窗口已关闭，可能是同名的新窗口。确认后将恢复记录的位置和最小化状态。"
                alert.addButton(withTitle: "恢复这个窗口"); alert.addButton(withTitle: "取消")
                guard alert.runModal() == .alertFirstButtonReturn else { return }
                self.target = recovered
                self.releaseTarget { _ in }
            }
        }
    }

    func poll() {
        updateInputPassthrough()
        guard target != nil, !busy else { return }
        let pointer = NSEvent.mouseLocation, now = ProcessInfo.processInfo.systemUptime
        let entry = headerRect.insetBy(dx: -8, dy: -8).contains(pointer)
        let content = isOpen && panel.frame.insetBy(dx: -8, dy: -8).contains(pointer)
        let suspended = dragging || moving || chooser?.isVisible == true || NSApp.modalWindow != nil
        switch policy.step(now: now, entry: entry, content: content, mouseDown: NSEvent.pressedMouseButtons != 0,
                           suspended: suspended, open: isOpen, pinned: pinned) {
        case .reveal: reveal()
        case .tuck: tuck()
        case .none: break
        }
        // Background frame reads also track independent user move/resize without snapping it back.
        if isOpen && !busy && !suspended && now - lastRead > 0.3, let target {
            lastRead = now; let token = serial, top = primaryTop
            perform(target, work: { try target.frame(primaryTop: top) }) { result in
                guard token == self.serial, self.isOpen, !self.dragging, !self.moving else { return }
                if case .success(let frame) = result, NSEvent.pressedMouseButtons == 0,
                   self.currentFrame != frame {
                    self.currentFrame = frame
                    self.configureChrome(around: frame)
                }
            }
        }
    }
    func startDrag(at point: CGPoint) {
        guard !busy else { return }
        dragging = true; dragStart = point; headerStart = panel.frame; policy.reset()
    }
    func drag(to point: CGPoint) {
        guard dragging else { return }
        var next = headerStart; next.origin.x += point.x - dragStart.x
        next = DrawerGeometry.clamp(next, to: screen.frame)
        let delta = next.minX - panel.frame.minX
        panel.setFrame(next, display: true)
        if isOpen && target != nil {
            currentFrame.origin.x += delta
            pendingMove = currentFrame
            flushMove()
        }
    }
    func endDrag() { dragging = false; policy.reset(); flushMove() }
    func flushMove() {
        guard !moving, let next = pendingMove, let target, !busy else { return }
        pendingMove = nil; moving = true
        let token = serial, top = primaryTop
        perform(target, work: {
            try target.setFrame(next, primaryTop: top, resize: false)
            return try target.frame(primaryTop: top)
        }) { result in
            self.moving = false
            guard token == self.serial else { return }
            if case .failure(let error) = result { self.pinned = true; self.showError(error) }
            if case .success(let actual) = result, self.pendingMove == nil {
                self.currentFrame = actual
                self.configureChrome(around: actual)
            }
            self.flushMove()
        }
    }
    @objc func screensChanged() {
        guard panel != nil else { return }
        let workScreen = screen
        panel.setFrame(DrawerGeometry.clamp(panel.frame, to: workScreen.frame), display: true)
        policy.reset()
        if isOpen { pinned = false; tuck() }
        else { configureChrome(around: nil) }
    }
    @objc func spaceChanged() {
        // Do not chase the foreign window across Spaces or activate its app.
        policy.reset(requireExit: true)
        if isOpen { pinned = false; tuck() }
    }
    @objc func appActivated(_ notification: Notification) {
        guard isOpen, !busy, let target,
              let activated = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              activated.processIdentifier != target.pid, activated.processIdentifier != getpid() else { return }
        // An unrelated foreground app could otherwise show through the native input aperture.
        pinned = false; tuck()
    }
    func showError(_ error: Error) {
        refreshHeader(error.localizedDescription)
        chooserStatus?.stringValue = error.localizedDescription
    }

    @objc func useSample() {
        guard !busy else { return }
        if sample == nil {
            let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 740, height: 430),
                                  styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
            window.title = "抽屉示例 · 可以直接输入和滚动"; window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: .darkAqua)
            window.minSize = CGSize(width: 460, height: 300)
            let content = window.contentView!
            let title = label("这是一个真正可操作的窗口", size: 23)
            title.font = .systemFont(ofSize: 23, weight: .semibold)
            title.frame = CGRect(x: 28, y: 367, width: 650, height: 36); title.autoresizingMask = [.width, .minYMargin]
            content.addSubview(title)
            let hint = label("在下面输入文字，点按钮，或拖动窗口。鼠标移开会自动收起。", size: 12, color: .secondaryLabelColor)
            hint.frame = CGRect(x: 28, y: 334, width: 650, height: 25); hint.autoresizingMask = [.width, .minYMargin]
            content.addSubview(hint)
            let scroll = NSScrollView(frame: CGRect(x: 28, y: 76, width: 684, height: 244))
            scroll.autoresizingMask = [.width, .height]; scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
            let text = NSTextView(frame: scroll.bounds)
            text.textColor = .textColor; text.backgroundColor = .textBackgroundColor
            text.isRichText = false; text.font = .systemFont(ofSize: 16); text.textContainerInset = CGSize(width: 14, height: 14)
            text.isVerticallyResizable = true; text.autoresizingMask = [.width]
            text.string = "把你的想法记在这里。\n\n试试这几个动作：\n1. 在这里输入中文或英文。\n2. 鼠标移到窗口外，等待自动收起。\n3. 再悬停左上方的黑色入口。\n4. 点击「固定」，让窗口一直展开。\n5. 拖动黑色顶栏，观察窗口跟随。\n\n这是内置示例。选择其他 App 的真实窗口，需要开启辅助功能权限。\n\n" + Array(repeating: "还可以滚动查看长内容。", count: 12).joined(separator: "\n")
            scroll.documentView = text; content.addSubview(scroll)
            let increment = button("点我 +1", action: #selector(incrementSample))
            increment.frame = CGRect(x: 28, y: 23, width: 110, height: 32); content.addSubview(increment)
            sampleEditLabel = label("已点击 0 次 · 收起后内容会保留", size: 12, color: .secondaryLabelColor)
            sampleEditLabel!.frame = CGRect(x: 155, y: 27, width: 450, height: 25); content.addSubview(sampleEditLabel!)
            window.center(); sample = window
        }
        sample!.makeKeyAndOrderFront(nil)
        bind(WindowTarget(local: sample!))
    }
    @objc func incrementSample() {
        sampleEdits += 1; sampleEditLabel?.stringValue = "已点击 \(sampleEdits) 次 · 收起后内容会保留"
    }
    @objc func quit() { NSApp.terminate(nil) }
    @objc func showHeader() {
        chooser?.orderOut(nil)
        panel.makeKeyAndOrderFront(nil)
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard target != nil else { return .terminateNow }
        guard !busy else { refreshHeader("窗口操作进行中，请稍后再退出。"); return .terminateCancel }
        DispatchQueue.main.async {
            self.releaseTarget { success in
                if success { NSApp.reply(toApplicationShouldTerminate: true) }
                else {
                    let alert = NSAlert(); alert.messageText = "窗口暂时未恢复"
                    alert.informativeText = "可以返回重试，或退出后从 Dock 恢复窗口。下次启动 Demo 也可点恢复上次窗口。"
                    alert.addButton(withTitle: "返回重试"); alert.addButton(withTitle: "退出")
                    NSApp.reply(toApplicationShouldTerminate: alert.runModal() == .alertSecondButtonReturn)
                }
            }
        }
        return .terminateLater
    }
    func runSmoke() {
        // Exercises the real AppKit sample lifecycle. No foreign app or permissions involved.
        useSample()
        precondition(target != nil && !isOpen)
        precondition(!sample!.isVisible && !sample!.isMiniaturized)
        pinned = true
        reveal()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            fputs("Hosted native geometry: shell=\(self.panel.frame), header=\(self.headerRect), window=\(self.currentFrame)\n", stderr)
            precondition(self.isOpen && self.sample!.isVisible)
            precondition(self.panel.frame.contains(self.currentFrame))
            precondition(self.currentFrame.maxY < self.headerRect.minY)
            precondition(self.header.aperture == DrawerGeometry.localAperture(window: self.currentFrame, shell: self.panel.frame))
            self.exportHostedPreview()
            self.incrementSample(); self.pinned = true
            self.poll(); precondition(self.isOpen)
            let start = self.panel.frame.center
            self.startDrag(at: start)
            self.drag(to: CGPoint(x: start.x + 70, y: start.y))
            self.endDrag()
            precondition(abs(self.sample!.frame.minX - self.currentFrame.minX) < 1)
            self.pinned = false; self.tuck()
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                precondition(!self.isOpen)
                precondition(!self.sample!.isVisible && !self.sample!.isMiniaturized)
                precondition(self.header.aperture == nil && abs(self.panel.frame.height - self.headerHeight) < 1)
                self.exportCollapsedPreview()
                self.pinned = true
                self.reveal()
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                    precondition(self.isOpen && self.sampleEdits == 1)
                    self.releaseTarget { success in
                        precondition(success && self.target == nil && self.sample!.isVisible)
                        precondition(self.header.aperture == nil && abs(self.panel.frame.height - self.headerHeight) < 1)
                        print("PASS: native body containment, transparent input aperture, bind, reveal, pin, follow placement, top-bar collapse WITHOUT minimizing, re-reveal, preserved content, release/restore")
                        NSApp.terminate(nil)
                    }
                }
            }
        }
    }

    func exportCollapsedPreview() {
        guard let path = ProcessInfo.processInfo.environment["CODEPET_DRAWER_COLLAPSED_PREVIEW"],
              let bitmap = header.bitmapImageRepForCachingDisplay(in: header.bounds) else { return }
        header.cacheDisplay(in: header.bounds, to: bitmap)
        if let png = bitmap.representation(using: .png, properties: [:]) {
            try! png.write(to: URL(fileURLWithPath: path))
        }
    }

    func exportHostedPreview() {
        guard let sample, let frameView = sample.contentView?.superview,
              let shellBitmap = header.bitmapImageRepForCachingDisplay(in: header.bounds),
              let sampleBitmap = frameView.bitmapImageRepForCachingDisplay(in: frameView.bounds),
              let aperture = header.aperture else { preconditionFailure("native view rendering unavailable") }
        header.cacheDisplay(in: header.bounds, to: shellBitmap)
        frameView.cacheDisplay(in: frameView.bounds, to: sampleBitmap)
        let centerX = Int(aperture.midX * CGFloat(shellBitmap.pixelsWide) / header.bounds.width)
        let centerY = Int(aperture.midY * CGFloat(shellBitmap.pixelsHigh) / header.bounds.height)
        // Pixel coordinates may be flipped; the aperture center is tested along with its reflected pixel.
        let centerAlphas = [centerY, shellBitmap.pixelsHigh - 1 - centerY].compactMap {
            shellBitmap.colorAt(x: centerX, y: max(0, min(shellBitmap.pixelsHigh - 1, $0)))?.alphaComponent
        }
        precondition(centerAlphas.contains { $0 < 0.01 }, "the native window aperture must remain transparent")
        guard let path = ProcessInfo.processInfo.environment["CODEPET_DRAWER_PREVIEW"] else { return }
        let image = NSImage(size: header.bounds.size)
        let sampleImage = NSImage(size: frameView.bounds.size)
        sampleImage.addRepresentation(sampleBitmap)
        let shellImage = NSImage(size: header.bounds.size)
        shellImage.addRepresentation(shellBitmap)
        image.lockFocus()
        sampleImage.draw(in: aperture, from: .zero, operation: .sourceOver, fraction: 1)
        shellImage.draw(in: header.bounds, from: .zero, operation: .sourceOver, fraction: 1)
        image.unlockFocus()
        if let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
           let png = bitmap.representation(using: .png, properties: [:]) {
            try! png.write(to: URL(fileURLWithPath: path))
            print("Native-view composite (not a desktop screenshot): \(path)")
        }
    }
}

extension CGRect {
    var center: CGPoint { CGPoint(x: midX, y: midY) }
    var area: CGFloat { isNull ? 0 : max(0, width) * max(0, height) }
}
let app = NSApplication.shared
let delegate = DrawerDemo()
app.delegate = delegate
app.run()
