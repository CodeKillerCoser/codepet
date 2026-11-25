"""Portable entry: interpreter and dependencies are adjacent, never user-installed."""
from pathlib import Path
import site
import sys

root = Path(__file__).resolve().parent
site.addsitedir(str(root / "packages"))
sys.path.insert(0, str(root))

from codepet_recall.server import main

if __name__ == "__main__":
    main()
