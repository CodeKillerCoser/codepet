"""Repair the bounded TURN reservation budget without changing auth or listeners."""
from pathlib import Path
import datetime
import re
import shutil
import subprocess

LIMITS = {'user-quota': 8, 'total-quota': 20, 'max-bps': 500000, 'bps-capacity': 10000000}

def configure(text):
    assert LIMITS['total-quota'] * LIMITS['max-bps'] <= LIMITS['bps-capacity']
    for key, value in LIMITS.items():
        pattern = rf'(?m)^{key}=\d+$'
        if len(re.findall(pattern, text)) != 1:
            raise ValueError(f'Expected one {key} setting')
        text = re.sub(pattern, f'{key}={value}', text)
    return text

def deploy():
    path = Path('/etc/coturn/turnserver.conf')
    old = path.read_text()
    updated = configure(old)
    backup = Path('/opt/codepet-signal/backups') / ('turn-capacity-' + datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ'))
    backup.mkdir(mode=0o700, parents=True)
    shutil.copy2(path, backup/path.name)
    try:
        path.write_text(updated)
        subprocess.run(['systemctl', 'restart', 'coturn'], check=True)
        subprocess.run(['systemctl', 'is-active', '--quiet', 'coturn'], check=True)
    except Exception:
        path.write_text(old)
        subprocess.run(['systemctl', 'restart', 'coturn'], check=True)
        raise
    print('TURN reservation budget repaired; backup:', backup)
    print('Limits:', LIMITS)

if __name__ == '__main__':
    deploy()
