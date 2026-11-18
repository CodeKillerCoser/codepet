#!/bin/zsh
set -euo pipefail
task_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
task_root="$(cd -- "$task_dir/../.." && pwd)"
task_output="$task_root/artifacts/window-drawer-demo"
task_app="$task_output/Code Pet Drawer.app"
mkdir -p "$task_app/Contents/MacOS"
xcrun swiftc -swift-version 5 -O "$task_dir/DrawerPolicy.swift" "$task_dir/main.swift" -o "$task_app/Contents/MacOS/CodePetDrawer" -framework AppKit -framework ApplicationServices
cat > "$task_app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>CodePetDrawer</string>
<key>CFBundleIdentifier</key><string>com.codepet.window-drawer-demo</string>
<key>CFBundleName</key><string>Code Pet Drawer</string>
<key>CFBundleDisplayName</key><string>Code Pet Drawer</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.3.0</string>
<key>CFBundleVersion</key><string>3</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$task_app"
xcrun swiftc -swift-version 5 "$task_dir/DrawerPolicy.swift" "$task_dir/LogicTests.swift" -o "$task_output/logic-tests"
"$task_output/logic-tests"
printf 'Demo built: %s\n' "$task_app"
if [[ "${1:-}" == "--open" ]]; then open "$task_app"; fi
