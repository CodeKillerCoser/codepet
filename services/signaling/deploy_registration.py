"""Incrementally deploy enrollment on an authorized server, preserving proxy/TURN/VPN."""
import datetime
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.request

def deploy(source):
    source = Path(source)
    python = '/opt/codepet-venv/bin/python'
    subprocess.run([python, '-m', 'pip', 'install', '-r', str(source / 'requirements.txt')], check=True)
    subprocess.run([python, '-m', 'unittest', 'test_server', 'test_registration', '-q'], cwd=source, check=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    backup = Path('/opt/codepet-signal/backups') / ('registration-' + stamp)
    backup.mkdir(mode=0o700, parents=True)
    config_path = Path('/etc/codepet-signal/config.json')
    config = json.loads(config_path.read_text())
    with sqlite3.connect(config['database']) as src, sqlite3.connect(backup/'signal.db') as dst:
        src.backup(dst)
    targets = [Path('/opt/codepet-signal/server.py'), Path('/opt/codepet-signal/registration.py'), config_path]
    originals = {}
    for target in targets:
        originals[target] = target.read_bytes() if target.exists() else None
        if target.exists(): shutil.copy2(target, backup / target.name)
    try:
        for name in ['registration.py', 'server.py']:
            Path('/opt/codepet-signal',name).write_bytes((source/name).read_bytes())
            Path('/opt/codepet-signal',name).chmod(0o644)
        config['deviceRegistration'] = True
        config.setdefault('maxDevices', 1024)
        config_path.write_text(json.dumps(config))
        subprocess.run(['systemctl','restart','codepet-signal'], check=True)
        for n in range(20):
            try:
                with urllib.request.urlopen('http://127.0.0.1:8787/health', timeout=2) as response:
                    assert json.load(response)['deviceRegistration'] is True
                break
            except (OSError, AssertionError, KeyError):
                if n == 19: raise
                time.sleep(0.25)
    except Exception:
        for target, data in originals.items():
            if data is None: target.unlink(missing_ok=True)
            else: target.write_bytes(data)
        subprocess.run(['systemctl','restart','codepet-signal'], check=True)
        raise
    print('Enrollment enabled; backup:', backup)

if __name__ == '__main__': deploy(sys.argv[1])
