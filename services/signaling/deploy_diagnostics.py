"""Root-only incremental diagnostics deployment; preserve identities and ICE policy.

Usage on the VPS: python3 deploy_diagnostics.py /path/to/new/server.py
Backups are kept next to each replaced file; health failure restores all three.
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


def deploy(source):
    py_compile.compile(str(source), doraise=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    server = Path('/opt/codepet-signal/server.py')
    turn = Path('/etc/coturn/turnserver.conf')
    rotation = Path('/etc/logrotate.d/coturn')
    targets = [server, turn, rotation]
    backups = {}
    for target in targets:
        backup = target.with_name(target.name + '.before-diagnostics-' + stamp)
        shutil.copy2(target, backup)
        backups[target] = backup
    try:
        # Writing existing files retains service ownership and permissions.
        server.write_bytes(source.read_bytes())
        lines = [line for line in turn.read_text().splitlines()
                 if line.split('=', 1)[0] not in {'log-file', 'simple-log', 'log-binding', 'verbose'}]
        lines += ['log-file=/var/log/coturn/turnserver.log', 'simple-log', 'log-binding', 'verbose']
        subprocess.run(['install', '-d', '-o', 'coturn', '-g', 'coturn', '-m', '750', '/var/log/coturn'], check=True)
        turn.write_text('\n'.join(lines) + '\n')
        rotation.write_text('''/var/log/coturn/turnserver.log {
    daily
    maxsize 10M
    rotate 3
    missingok
    notifempty
    compress
    delaycompress
    copytruncate
    su coturn coturn
}
''')
        subprocess.run(['logrotate', '--debug', str(rotation)], check=True, capture_output=True)
        subprocess.run(['systemctl', 'restart', 'codepet-signal', 'coturn'], check=True)
        subprocess.run(['systemctl', 'is-active', '--quiet', 'codepet-signal', 'coturn'], check=True)
        for attempt in range(20):
            try:
                with urllib.request.urlopen('http://127.0.0.1:8787/health', timeout=2) as response:
                    assert response.status == 200
                break
            except OSError:
                if attempt == 19:
                    raise
                time.sleep(0.25)
        # A local STUN binding validates the listener and produces a diagnostic entry.
        ip = next(line.split('=', 1)[1] for line in lines if line.startswith('listening-ip='))
        transaction = os.urandom(12)
        packet = struct.pack('!HHI', 1, 0, 0x2112A442) + transaction
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(5)
            sock.sendto(packet, (ip, 3478))
            reply = sock.recv(1024)
            assert reply[:2] == b'\x01\x01' and reply[8:20] == transaction
        print('Diagnostics deployed; signal health and STUN binding passed; backup suffix:', stamp)
    except BaseException:
        for target, backup in backups.items():
            shutil.copy2(backup, target)
        subprocess.run(['systemctl', 'restart', 'codepet-signal', 'coturn'], check=False)
        raise


if __name__ == '__main__':
    deploy(Path(sys.argv[1]))
