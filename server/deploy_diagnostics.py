"""Root-only incremental diagnostics deployment; preserve identities and ICE policy.

Usage on the VPS: python3 deploy_diagnostics.py /path/to/new/server.py
Backups stay outside service include directories; health failure restores all three.
"""
import datetime
import os
from pathlib import Path
import py_compile
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.request


def backup_targets(targets, directory):
    directory.mkdir(parents=True, exist_ok=False, mode=0o700)
    backups = {}
    for target in targets:
        backup = directory / target.relative_to('/')
        backup.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(target, backup)
        backups[target] = backup
    return backups


def deploy(source):
    # The runtime now has policy/registration modules; updating server.py alone
    # is unsafe. Keep the legacy command routed through the tested bundle.
    from deploy_management import deploy as deploy_bundle
    deploy_bundle(Path(source).parent)


if __name__ == '__main__':
    deploy(Path(sys.argv[1]))
