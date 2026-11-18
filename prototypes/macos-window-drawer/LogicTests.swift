import Foundation
import CoreGraphics

@main struct LogicTests {
    static func main() {
        var policy = DrawerPolicy()
        func step(_ time: Double, entry: Bool = false, content: Bool = false,
                  down: Bool = false, suspended: Bool = false, open: Bool = false,
                  pinned: Bool = false) -> DrawerPolicy.Action {
            policy.step(now: time, entry: entry, content: content, mouseDown: down,
                        suspended: suspended, open: open, pinned: pinned)
        }
        precondition(step(0, entry: true) == .none)
        precondition(step(0.1) == .none) // A brief crossing must not open.
        precondition(step(0.3, entry: true) == .none)
        precondition(step(0.51, entry: true) == .reveal)
        precondition(step(1, content: true, open: true) == .none)
        precondition(step(1.1, open: true) == .none)
        precondition(step(1.5, content: true, open: true) == .none) // Re-entry cancels tuck.
        precondition(step(2, open: true) == .none)
        precondition(step(2.7, down: true, open: true) == .none) // Selection/drag pauses.
        precondition(step(3, open: true) == .none)
        precondition(step(3.7, open: true) == .tuck)
        precondition(step(4, open: true, pinned: true) == .none)
        precondition(step(5, suspended: true, open: true) == .none)
        precondition(step(5.1, open: true) == .none)
        precondition(step(5.76, open: true) == .tuck)
        policy.reset(requireExit: true)
        precondition(step(6, entry: true) == .none)
        precondition(step(7, entry: true) == .none)
        precondition(step(8) == .none)
        precondition(step(9, entry: true) == .none)
        precondition(step(9.21, entry: true) == .reveal)

        for frame in [CGRect(x: 100, y: 400, width: 600, height: 300),
                      CGRect(x: -900, y: 1300, width: 500, height: 600)] {
            let ax = DrawerGeometry.axRect(fromAppKit: frame, primaryTop: 1080)
            precondition(DrawerGeometry.appKitRect(fromAX: ax, primaryTop: 1080) == frame)
        }
        let work = CGRect(x: -1440, y: -100, width: 1440, height: 900)
        let clamped = DrawerGeometry.clamp(CGRect(x: -2000, y: 700, width: 600, height: 400), to: work)
        precondition(work.contains(clamped))
        for screen in [CGRect(x: 0, y: 0, width: 1440, height: 900),
                       CGRect(x: -1920, y: 900, width: 1920, height: 1080)] {
            for x in [screen.minX, screen.midX, screen.maxX] {
                let layout = DrawerGeometry.hostedLayout(
                    anchor: CGRect(x: x - 260, y: screen.maxY - 80, width: 520, height: 80),
                    windowSize: CGSize(width: 740, height: 420), screen: screen, headerHeight: 80)
                precondition(layout.shell.contains(layout.window))
                precondition(layout.window.maxY < layout.header.minY)
                precondition(layout.shell.maxY == screen.maxY)
                precondition(layout.shell.minX >= screen.minX && layout.shell.maxX <= screen.maxX)
                let hole = DrawerGeometry.localAperture(window: layout.window, shell: layout.shell)
                precondition(hole.minX == DrawerGeometry.padding && hole.minY == DrawerGeometry.padding)
                precondition(DrawerGeometry.receivesNativeInput(pointer: CGPoint(x: layout.window.midX, y: layout.window.midY),
                                                              window: layout.window, open: true))
                precondition(!DrawerGeometry.receivesNativeInput(pointer: CGPoint(x: layout.header.midX, y: layout.header.midY),
                                                               window: layout.window, open: true))
            }
        }
        print("PASS: hover dwell, re-entry, drag, pin, suspension, explicit-close suppression, multi-screen coordinates")
        print("PASS: hosted body containment, header separation, edge alignment, native input aperture")
        let displays = [CGRect(x: 0, y: 0, width: 1440, height: 900),
                        CGRect(x: -1920, y: 900, width: 1920, height: 1080)]
        for candidate in DrawerGeometry.parkingCandidates(size: CGSize(width: 800, height: 600), screens: displays) {
            precondition(DrawerGeometry.isOutsideDisplays(candidate, screens: displays))
        }
        precondition(!DrawerGeometry.isOutsideDisplays(CGRect(x: -1000, y: 1200, width: 800, height: 500), screens: displays))
        precondition(!DrawerGeometry.isOutsideDisplays(CGRect(x: 1439, y: 200, width: 800, height: 500), screens: displays))
        precondition(!DrawerGeometry.isOutsideDisplays(.zero, screens: displays))
        precondition(!DrawerGeometry.isOutsideDisplays(CGRect(x: 2000, y: 2000, width: 800, height: 600), screens: []))
        print("PASS: no parking on a secondary display, no visible-edge acceptance, invalid parking rejected")
    }
}
