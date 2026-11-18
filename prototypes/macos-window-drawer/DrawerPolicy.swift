import Foundation
import CoreGraphics

// Pure hover policy. The native coordinator supplies absolute screen hit testing.
struct DrawerPolicy {
    enum Action { case none, reveal, tuck }
    var enterDelay = 0.2
    var leaveDelay = 0.65
    var enteredAt: TimeInterval?
    var leftAt: TimeInterval?
    var requireExit = false

    mutating func reset(requireExit: Bool = false) {
        enteredAt = nil
        leftAt = nil
        self.requireExit = requireExit
    }

    mutating func step(now: TimeInterval, entry: Bool, content: Bool,
                       mouseDown: Bool, suspended: Bool, open: Bool, pinned: Bool) -> Action {
        if requireExit {
            if !entry { requireExit = false }
            return .none
        }
        if suspended || mouseDown {
            enteredAt = nil; leftAt = nil
            return .none
        }
        if !open {
            leftAt = nil
            guard entry else { enteredAt = nil; return .none }
            if enteredAt == nil { enteredAt = now }
            if now - enteredAt! >= enterDelay { enteredAt = nil; return .reveal }
        } else {
            enteredAt = nil
            guard !pinned && !entry && !content else { leftAt = nil; return .none }
            if leftAt == nil { leftAt = now }
            if now - leftAt! >= leaveDelay { leftAt = nil; return .tuck }
        }
        return .none
    }
}

enum DrawerGeometry {
    struct HostedLayout {
        let shell: CGRect
        let window: CGRect
        let header: CGRect
    }
    static let padding: CGFloat = 20
    static let bodyGap: CGFloat = 12
    static let collapsedWidth: CGFloat = 520

    static func parkingCandidates(size: CGSize, screens: [CGRect]) -> [CGRect] {
        guard let first = screens.first else { return [] }
        let union = screens.dropFirst().reduce(first) { $0.union($1) }
        // Try fully outside every physical display, never on an adjacent display.
        return [CGRect(x: union.midX - size.width / 2, y: union.maxY + 80, width: size.width, height: size.height),
                CGRect(x: union.minX - size.width - 80, y: union.midY, width: size.width, height: size.height),
                CGRect(x: union.maxX + 80, y: union.midY, width: size.width, height: size.height),
                CGRect(x: union.midX - size.width / 2, y: union.minY - size.height - 80, width: size.width, height: size.height)]
    }
    static func isOutsideDisplays(_ frame: CGRect, screens: [CGRect]) -> Bool {
        !screens.isEmpty && !frame.isNull && frame.width > 0 && frame.height > 0 &&
        frame.origin.x.isFinite && frame.origin.y.isFinite && frame.width.isFinite && frame.height.isFinite &&
        screens.allSatisfy { !$0.intersects(frame) }
    }

    // Window geometry is inside the black shell, not a detached window below a small toolbar.
    static func hostedLayout(anchor: CGRect, windowSize: CGSize, screen: CGRect,
                             headerHeight: CGFloat) -> HostedLayout {
        let width = min(screen.width, max(collapsedWidth, windowSize.width + 2 * padding))
        let x = min(max(anchor.midX - width / 2, screen.minX), screen.maxX - width)
        let top = screen.maxY
        let window = CGRect(x: x + padding, y: top - headerHeight - bodyGap - windowSize.height,
                            width: windowSize.width, height: windowSize.height)
        return HostedLayout(shell: CGRect(x: x, y: window.minY - padding, width: width,
                                          height: headerHeight + bodyGap + window.height + padding),
                            window: window,
                            header: CGRect(x: x, y: top - headerHeight, width: width, height: headerHeight))
    }
    static func localAperture(window: CGRect, shell: CGRect) -> CGRect {
        window.offsetBy(dx: -shell.minX, dy: -shell.minY)
    }
    static func receivesNativeInput(pointer: CGPoint, window: CGRect, open: Bool) -> Bool {
        // Keep the original window's native resize hit area usable too.
        open && window.insetBy(dx: -4, dy: -4).contains(pointer)
    }
    static func axRect(fromAppKit rect: CGRect, primaryTop: CGFloat) -> CGRect {
        CGRect(x: rect.minX, y: primaryTop - rect.maxY, width: rect.width, height: rect.height)
    }
    static func appKitRect(fromAX rect: CGRect, primaryTop: CGFloat) -> CGRect {
        CGRect(x: rect.minX, y: primaryTop - rect.maxY, width: rect.width, height: rect.height)
    }
    static func clamp(_ frame: CGRect, to work: CGRect) -> CGRect {
        CGRect(x: min(max(frame.minX, work.minX), max(work.minX, work.maxX - frame.width)),
               y: min(max(frame.minY, work.minY), max(work.minY, work.maxY - frame.height)),
               width: frame.width, height: frame.height)
    }
}
