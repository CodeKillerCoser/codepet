#!/usr/bin/env python3

import platform
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
APP_PATH = ROOT / "src-tauri/target/release/bundle/macos/Code Pet.app"
DMG_DIR = ROOT / "src-tauri/target/release/bundle/dmg"


def run(command: list[str]) -> None:
    result = subprocess.run(command, cwd=ROOT, env=None)
    if result.returncode != 0:
        raise SystemExit(result.returncode)


def remove_old_dmg_files() -> None:
    if not DMG_DIR.exists():
        return

    for dmg_path in DMG_DIR.glob("*.dmg"):
        dmg_path.unlink()


def find_created_dmg() -> Path:
    dmg_files = sorted(DMG_DIR.glob("*.dmg"), key=lambda path: path.stat().st_mtime, reverse=True)
    if not dmg_files:
        print(f"DMG was not created in: {DMG_DIR}", file=sys.stderr)
        raise SystemExit(1)
    return dmg_files[0]


def main() -> int:
    if platform.system() != "Darwin":
        print("package:signed currently creates and verifies macOS DMG packages only.", file=sys.stderr)
        return 1

    remove_old_dmg_files()
    run(["npm", "run", "tauri", "build", "--", "--bundles", "dmg"])

    dmg_path = find_created_dmg()

    if APP_PATH.exists():
        run(["/usr/bin/codesign", "--verify", "--deep", "--strict", "--verbose=2", str(APP_PATH)])
        run(["/usr/bin/codesign", "-dv", str(APP_PATH)])

    run(["/usr/bin/codesign", "--verify", "--verbose=2", str(dmg_path)])
    run(["/usr/bin/codesign", "-dv", str(dmg_path)])

    print(f"DMG ready: {dmg_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
