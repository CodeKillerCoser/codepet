"""Deny mutation of the production ledger, including accidental library writes."""

import os
from pathlib import Path
import sys


class ReadOnlyLedger:
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.blocked_writes = 0

    def contains(self, value) -> bool:
        if not isinstance(value, (str, bytes, os.PathLike)):
            return False
        path = Path(os.fsdecode(value)).resolve()
        return path == self.root or self.root in path.parents

    def audit(self, event, args):
        targets = []
        if event == "open":
            path, mode, flags = args
            writing = (isinstance(mode, str) and any(char in mode for char in "wax+"))
            writing = writing or bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND))
            if writing:
                targets = [path]
        elif event in {"os.remove", "os.rmdir", "os.mkdir", "os.chmod", "os.utime", "os.truncate"}:
            targets = args[:1]
        elif event in {"os.rename", "os.link", "os.symlink"}:
            targets = args[:2]
        if any(self.contains(target) for target in targets):
            self.blocked_writes += 1
            raise PermissionError("The Cognee probe cannot mutate the production Task ledger")

    def install(self):
        sys.addaudithook(self.audit)
