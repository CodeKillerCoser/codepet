"""Incremental VPS maintenance. Run from a staged source directory as root.

Existing identities, certificates, downloads and VPN configuration are preserved.
Only Code Pet's signal/nginx/TURN settings and a scoped nftables budget are changed.
"""
import datetime
import json
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import urllib.request

from config_source import (DEPLOYMENT, configure_signal, read_config, render, template_values, firewall_ports)

TURN_LIMITS = DEPLOYMENT['turn']
EGRESS_BYTES_PER_SECOND = DEPLOYMENT['egress']['bytesPerSecond']
RUNTIME_FILES = ('server.py', 'registration.py', 'policy.py')
PORT = f"{TURN_LIMITS['min-port']}-{TURN_LIMITS['max-port']}/udp"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def configure_turn(text):
    for key, value in TURN_LIMITS.items():
        pattern = rf'(?m)^{key}=\d+$'
        if len(re.findall(pattern, text)) != 1:
            raise ValueError('Expected one TURN setting: ' + key)
        text = re.sub(pattern, f'{key}={value}', text)
    return text


def configure_nginx(text):
    zone = r'limit_req_zone \$binary_remote_addr zone=codepet_ip:10m rate=\d+r/s;'
    location = r'location\s+/\s*\{([^{}]*)\}'
    blocks = list(re.finditer(location, text))
    if len(re.findall(zone, text)) != 1 or len(blocks) != 1:
        raise ValueError('Unexpected nginx Code Pet proxy configuration')
    current = blocks[0][1]
    if 'proxy_pass http://127.0.0.1:8787;' not in current:
        raise ValueError('Unexpected signaling upstream')
    if ('proxy_set_header X-Real-IP' in current
            and 'proxy_set_header X-Real-IP $remote_addr;' not in current):
        raise ValueError('Refusing ambiguous proxy source configuration')
    desired = render('nginx.conf.tpl', **template_values())
    desired_zone = re.search(zone, desired).group(0)
    desired_location = re.search(location, desired).group(0)
    text = re.sub(zone, lambda _: desired_zone, text)
    text = re.sub(r'listen\s+\d+\s+ssl;', f"listen {DEPLOYMENT['httpsPort']} ssl;", text)
    return re.sub(location, lambda _: desired_location, text)


def budget_rules(rate=EGRESS_BYTES_PER_SECOND):
    if type(rate) is not int or rate <= 0:
        raise ValueError('Invalid egress budget')
    burst = DEPLOYMENT['egress']['burstBytes']
    if rate != EGRESS_BYTES_PER_SECOND:
        burst = min(burst, max(1000, rate // 5))  # Isolated probe override only.
    return render('turn-budget.nft.tpl', EGRESS_RATE=rate, EGRESS_BURST=burst)


BUDGET_UNIT = read_config('codepet-turn-budget.service')
TURN_DEPENDENCY = read_config('coturn-budget.conf')


def healthy():
    for attempt in range(20):
        try:
            with urllib.request.urlopen('http://127.0.0.1:8787/health', timeout=2) as response:
                assert json.load(response)['deviceRegistration'] is True
            return
        except (OSError, AssertionError, KeyError):
            if attempt == 19:
                raise
            time.sleep(0.25)


def deploy(source):
    source = Path(source).resolve()
    if source != Path(__file__).resolve().parent:
        raise ValueError('Run the deploy script from the source bundle being deployed')
    run('/opt/codepet-venv/bin/python', '-m', 'unittest', 'discover', '-s', str(source / 'tests'), '-q', cwd=source)
    for name in RUNTIME_FILES:
        if not (source / name).is_file():
            raise ValueError('Missing runtime source: ' + name)
    config_path = Path('/etc/codepet-signal/config.json')
    config = configure_signal(json.loads(config_path.read_text()))
    targets = {
        **{Path('/opt/codepet-signal', name): (source / name).read_bytes() for name in RUNTIME_FILES},
        config_path: (json.dumps(config, indent=2) + '\n').encode(),
        Path('/etc/coturn/turnserver.conf'): configure_turn(Path('/etc/coturn/turnserver.conf').read_text()).encode(),
        Path('/etc/nginx/conf.d/codepet-signal.conf'): configure_nginx(Path('/etc/nginx/conf.d/codepet-signal.conf').read_text()).encode(),
        Path('/etc/codepet-signal/turn-budget.nft'): budget_rules().encode(),
        Path('/etc/systemd/system/codepet-turn-budget.service'): BUDGET_UNIT.encode(),
        Path('/etc/systemd/system/coturn.service.d/codepet-budget.conf'): TURN_DEPENDENCY.encode(),
        **{Path('/etc/systemd/system', name): read_config(name).encode()
           for name in ('codepet-signal.service', 'codepet-cert-renew.service', 'codepet-cert-renew.timer')},
        Path('/etc/logrotate.d/coturn'): read_config('coturn.logrotate').encode(),
        Path('/etc/letsencrypt/renewal-hooks/deploy/codepet-reload.sh'): render(
            'codepet-reload.sh.tpl', **template_values(config['publicIp'])).encode(),
    }
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    backup = Path('/opt/codepet-signal/backups') / ('management-' + stamp)
    backup.mkdir(mode=0o700, parents=True)
    originals = {}
    for path in targets:
        original = backup / path.relative_to('/')
        originals[path] = original if path.exists() else None
        if path.exists():
            original.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, original)
    (backup / 'manifest.json').write_text(json.dumps({str(k): str(v) if v else None for k, v in originals.items()}))
    with sqlite3.connect(config['database']) as src, sqlite3.connect(backup / 'signal.db') as dst:
        src.backup(dst)
    old_nft = subprocess.run(['nft', 'list', 'table', 'inet', 'codepet_turn_budget'], capture_output=True, text=True)
    if old_nft.returncode == 0:
        (backup / 'turn-budget-previous.nft').write_text(old_nft.stdout)
    was_enabled = subprocess.run(['systemctl', 'is-enabled', '--quiet', 'codepet-turn-budget']).returncode == 0
    port_present = {}
    for permanent in (False, True):
        options = ['--permanent'] if permanent else []
        for port in firewall_ports():
            port_present[permanent, port] = subprocess.run(['firewall-cmd', *options, '--query-port=' + port], capture_output=True).returncode == 0
    try:
        for path, data in targets.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)  # Existing permissions/owners stay intact.
            if originals[path] is None:
                path.chmod(0o750 if path.name == 'codepet-reload.sh' else 0o644)
        run('nginx', '-t')
        run('logrotate', '--debug', '/etc/logrotate.conf', capture_output=True)
        run('nft', '--check', '-f', '/etc/codepet-signal/turn-budget.nft')
        for (permanent, port), present in port_present.items():
            if not present:
                run('firewall-cmd', *(['--permanent'] if permanent else []), '--add-port=' + port)
        run('systemctl', 'daemon-reload')
        run('systemctl', 'enable', 'codepet-turn-budget')
        # Atomically apply the rules even on repeat deployment; no stop/start gap.
        run('systemctl', 'reload-or-restart', 'codepet-turn-budget')
        run('systemctl', 'restart', 'codepet-signal', 'coturn')
        run('systemctl', 'reload', 'nginx')
        healthy()
        for service in ('codepet-signal', 'coturn', 'nginx', 'xray', 'codepet-turn-budget'):
            run('systemctl', 'is-active', '--quiet', service)
    except BaseException:
        # Restore code/config, not the live database: do not discard registrations
        # that might have arrived during deployment. Schema did not change.
        if not was_enabled:
            subprocess.run(['systemctl', 'disable', '--now', 'codepet-turn-budget'], capture_output=True)
        for path, original in originals.items():
            if original is None:
                path.unlink(missing_ok=True)
            else:
                shutil.copy2(original, path)
        if old_nft.returncode == 0:
            restore = 'add table inet codepet_turn_budget\nflush table inet codepet_turn_budget\n' + old_nft.stdout
            subprocess.run(['nft', '-f', '-'], input=restore, text=True, check=True)
        else:
            subprocess.run(['nft', 'delete', 'table', 'inet', 'codepet_turn_budget'], capture_output=True)
        for (permanent, port), present in port_present.items():
            if not present:
                subprocess.run(['firewall-cmd', *(['--permanent'] if permanent else []), '--remove-port=' + port], capture_output=True)
        run('systemctl', 'daemon-reload')
        run('systemctl', 'restart', 'codepet-signal', 'coturn')
        run('systemctl', 'reload', 'nginx')
        raise
    print('Deployment verified. Backup:', backup)
    print('TURN limits:', TURN_LIMITS, 'actual egress bytes/s:', EGRESS_BYTES_PER_SECOND)


if __name__ == '__main__':
    deploy(sys.argv[1])
