#!/usr/bin/env python3

import platform
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
APP_PATH = ROOT / "src-tauri/target/release/bundle/macos/Code Pet.app"
DMG_DIR = ROOT / "src-tauri/target/release/bundle/dmg"
NOTARY_ENV_NAMES = [
    "CODE_PET_NOTARY_KEYCHAIN_PROFILE",
    "APPLE_NOTARY_KEYCHAIN_PROFILE",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_APP_SPECIFIC_PASSWORD",
    "APPLE_TEAM_ID",
    "APPLE_NOTARIZE_APPLE_ID",
    "APPLE_NOTARIZE_PWD",
    "APPLE_NOTARIZE_TEAM_ID",
]


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    result = subprocess.run(command, cwd=ROOT, env=env)
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


def notarization_args(env: dict[str, str]) -> list[str]:
    profile = env.get("CODE_PET_NOTARY_KEYCHAIN_PROFILE") or env.get("APPLE_NOTARY_KEYCHAIN_PROFILE")
    if profile:
        return ["--keychain-profile", profile]

    apple_id = env.get("APPLE_ID") or env.get("APPLE_NOTARIZE_APPLE_ID")
    apple_password = env.get("APPLE_PASSWORD") or env.get("APPLE_APP_SPECIFIC_PASSWORD") or env.get("APPLE_NOTARIZE_PWD")
    team_id = env.get("APPLE_TEAM_ID") or env.get("APPLE_NOTARIZE_TEAM_ID")
    if apple_id and apple_password and team_id:
        return ["--apple-id", apple_id, "--password", apple_password, "--team-id", team_id]

    print(
        "Missing notarization credentials. Set CODE_PET_NOTARY_KEYCHAIN_PROFILE "
        "or APPLE_ID, APPLE_PASSWORD/APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID "
        "(legacy APPLE_NOTARIZE_APPLE_ID, APPLE_NOTARIZE_PWD and APPLE_NOTARIZE_TEAM_ID are also supported).",
        file=sys.stderr,
    )
    raise SystemExit(2)


def has_notarization_credentials(env: dict[str, str]) -> bool:
    has_profile = bool(env.get("CODE_PET_NOTARY_KEYCHAIN_PROFILE") or env.get("APPLE_NOTARY_KEYCHAIN_PROFILE"))
    has_apple_id = bool(env.get("APPLE_ID") or env.get("APPLE_NOTARIZE_APPLE_ID"))
    has_password = bool(env.get("APPLE_PASSWORD") or env.get("APPLE_APP_SPECIFIC_PASSWORD") or env.get("APPLE_NOTARIZE_PWD"))
    has_team_id = bool(env.get("APPLE_TEAM_ID") or env.get("APPLE_NOTARIZE_TEAM_ID"))
    return has_profile or (has_apple_id and has_password and has_team_id)


def notary_env_with_shell_fallback(env: dict[str, str]) -> dict[str, str]:
    if has_notarization_credentials(env):
        return env

    result = subprocess.run(
        ["/bin/zsh", "-ic", "env"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return env

    merged = env.copy()
    for line in result.stdout.splitlines():
        name, separator, value = line.partition("=")
        if separator and name in NOTARY_ENV_NAMES and value:
            merged.setdefault(name, value)
    return merged


def notarize_dmg(dmg_path: Path, env: dict[str, str]) -> None:
    run(
        [
            "/usr/bin/xcrun",
            "notarytool",
            "submit",
            str(dmg_path),
            "--wait",
            *notarization_args(env),
        ],
        env=env,
    )
    run(["/usr/bin/xcrun", "stapler", "staple", str(dmg_path)], env=env)
    run(["/usr/bin/xcrun", "stapler", "validate", str(dmg_path)], env=env)
    run(["/usr/sbin/spctl", "-a", "-vvv", "-t", "open", "--context", "context:primary-signature", str(dmg_path)], env=env)


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
    notarize_dmg(dmg_path, notary_env_with_shell_fallback(os.environ.copy()))

    print(f"Notarized DMG ready: {dmg_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
