"""Export the approved artwork without changing its colors or composition.

Requires Pillow; macOS iconutil builds the ICNS container.
Optionally pass --remote-root to update CodePet Remote's Android icons.
"""

import argparse
from pathlib import Path
import shutil
import subprocess

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--remote-root", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = root / "assets/branding/codepet-icon-source.png"
    artwork = Image.open(source).convert("RGBA")
    if artwork.width != artwork.height:
        raise ValueError("App icon source must be square")
    icons = root / "src-tauri/icons"

    def export(path, size):
        path.parent.mkdir(parents=True, exist_ok=True)
        artwork.resize((size, size), Image.Resampling.LANCZOS).save(path)

    for name, size in [("icon.png", 1024), ("32x32.png", 32),
                       ("128x128.png", 128), ("128x128@2x.png", 256)]:
        export(icons / name, size)
    iconset = icons / "icon.iconset"
    for size in (16, 32, 128, 256, 512):
        export(iconset / f"icon_{size}x{size}.png", size)
        export(iconset / f"icon_{size}x{size}@2x.png", size * 2)
    artwork.resize((256, 256), Image.Resampling.LANCZOS).save(
        icons / "icon.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)]
    )
    subprocess.run(["iconutil", "-c", "icns", str(iconset),
                    "-o", str(icons / "icon.icns")], check=True)

    if args.remote_root:
        remote = args.remote_root.resolve()
        if not (remote / "android/app/src/main/AndroidManifest.xml").is_file():
            raise ValueError("--remote-root must point to CodePet Remote")
        remote_source = remote / "assets/branding" / source.name
        remote_source.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, remote_source)
        for density, size in [("mdpi", 48), ("hdpi", 72), ("xhdpi", 96),
                              ("xxhdpi", 144), ("xxxhdpi", 192)]:
            export(remote / "android/app/src/main/res" /
                   f"mipmap-{density}/ic_launcher.png", size)
    print("Exported Code Pet icons" + (" and Remote Android icons" if args.remote_root else ""))


if __name__ == "__main__":
    main()
