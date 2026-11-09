#!/usr/bin/env swift
// Read-only diagnostic matching platform/pet_surface.rs. Run in a logged-in GUI session.
import AppKit
import CoreGraphics
import Foundation

func rect(_ r: NSRect) -> [String: Double] {
    ["x": Double(r.origin.x), "y": Double(r.origin.y),
     "width": Double(r.width), "height": Double(r.height)]
}
func surfaceRect(_ r: NSRect) -> [String: Double] {
    ["x": Double(r.minX), "y": Double(-r.maxY),
     "width": Double(r.width), "height": Double(r.height)]
}
func json(_ value: Any) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]),
          let text = String(data: data, encoding: .utf8) else { return "null" }
    return text
}

var jsonOnly = false
var statePath = NSHomeDirectory() + "/Library/Application Support/code-pet/config/pet-surface.json"
var appPath = "/Applications/Code Pet.app"
var args = Array(CommandLine.arguments.dropFirst())
while !args.isEmpty {
    let arg = args.removeFirst()
    switch arg {
    case "--json": jsonOnly = true
    case "--surface-state", "--app":
        guard !args.isEmpty else { fputs("Missing value for \(arg)\n", stderr); exit(2) }
        let value = args.removeFirst()
        if arg == "--app" { appPath = value } else { statePath = value }
    case "--help", "-h":
        print("swift diagnose_notch_macos.swift [--json] [--surface-state <pet-surface.json>] [--app <Code Pet.app>]")
        exit(0)
    default: fputs("Unknown argument: \(arg)\n", stderr); exit(2)
    }
}

let screens = NSScreen.screens
guard !screens.isEmpty else {
    fputs("No NSScreen found. Run this script in the Mac's logged-in desktop session.\n", stderr)
    exit(1)
}
var saved: [String: Any] = [:]
if let data = FileManager.default.contents(atPath: statePath),
   let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
    // Only surface identity, not app settings, conversation data or credentials.
    for key in ["displayId", "mode"] { saved[key] = value[key] }
}
let bundle = Bundle(path: appPath)
var reports: [[String: Any]] = []
for (index, screen) in screens.enumerated() {
    let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber
    let id = number?.uint32Value
    let displayID = number?.stringValue ?? screen.localizedName
    let f = screen.frame
    var report: [String: Any] = [
        "id": displayID, "name": screen.localizedName,
        "isPrimary": index == 0, "isNSScreenMain": screen == NSScreen.main,
        "containsMouse": NSMouseInRect(NSEvent.mouseLocation, f, false),
        "matchesSavedDisplay": saved["displayId"] as? String == displayID,
        "scaleFactor": Double(screen.backingScaleFactor),
        "frameAppKitPoints": rect(f), "visibleFrameAppKitPoints": rect(screen.visibleFrame),
        "frameCodePetCoordinates": surfaceRect(f),
        "topReservedByVisibleFrame": Double(f.maxY - screen.visibleFrame.maxY),
        "builtIn": id.map { CGDisplayIsBuiltin($0) != 0 } as Any? ?? NSNull(),
    ]
    if #available(macOS 12.0, *) {
        let insets = screen.safeAreaInsets
        let left = screen.auxiliaryTopLeftArea
        let right = screen.auxiliaryTopRightArea
        let physicalNotch = insets.top > 0 // Exact current Code Pet predicate.
        let hasAreas = (left?.width ?? 0) > 0 && (right?.width ?? 0) > 0
        let gap = hasAreas ? Double(right!.minX - left!.maxX) : nil
        let x = physicalNotch && hasAreas ? left!.maxX : f.midX - 90
        let width = physicalNotch && hasAreas ? max(CGFloat(gap!), 1) : 180
        let height = physicalNotch ? insets.top : max(f.maxY - screen.visibleFrame.maxY, 24)
        report["safeAreaInsets"] = ["top": Double(insets.top), "left": Double(insets.left), "bottom": Double(insets.bottom), "right": Double(insets.right)]
        report["auxiliaryTopLeftArea"] = left.map(rect) as Any? ?? NSNull()
        report["auxiliaryTopRightArea"] = right.map(rect) as Any? ?? NSNull()
        report["auxiliaryGapWidth"] = gap as Any? ?? NSNull()
        report["codePetPhysicalNotch"] = physicalNotch
        report["codePetCanDock"] = true // The app creates a fallback target even without a notch.
        report["codePetDockTarget"] = ["x": Double(x), "y": Double(-f.maxY), "width": Double(width), "height": Double(height)]
        report["dockTargetSource"] = !physicalNotch ? "no-notch-top-fallback" : hasAreas ? "auxiliary-areas" : "notch-width-fallback-180pt"
        report["geometryConsistentWithNotch"] = physicalNotch && hasAreas && (gap ?? 0) > 0
    } else {
        report["codePetPhysicalNotch"] = NSNull()
        report["note"] = "macOS < 12: notch APIs unavailable; cannot reproduce current app detection."
    }
    reports.append(report)
}
let result: [String: Any] = [
    "os": ProcessInfo.processInfo.operatingSystemVersionString,
    "appPath": appPath,
    "appVersion": bundle?.object(forInfoDictionaryKey: "CFBundleShortVersionString") ?? NSNull(),
    "appSafeAreaCompatibilityPreference": bundle?.object(forInfoDictionaryKey: "NSPrefersDisplaySafeAreaCompatibilityMode") ?? "not-set",
    "savedSurfaceState": saved, "screens": reports,
    "scope": "Separate diagnostic process. Saved displayId is not the live menu/panel screen. App-specific compatibility mode may produce different geometry. All geometry uses AppKit points, not pixels.",
]
if !jsonOnly {
    print("Code Pet 刘海屏诊断（只读，不修改设置）")
    print("当前判断：physicalNotch = NSScreen.safeAreaInsets.top > 0")
    print("canDock 是吸附能力；无刘海屏也为 true，不能据此认定有物理刘海。")
    for screen in reports {
        print("屏幕 \(screen["id"]!): \(screen["name"]!) | physicalNotch=\(screen["codePetPhysicalNotch"]!) | \(screen["dockTargetSource"] ?? "unsupported")")
    }
    print("\n以下是原始检测值。保存的 displayId 仅供对照，不代表当前菜单实际所在屏幕。\n")
}
print(json(result))
