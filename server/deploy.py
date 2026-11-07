"""Initial AlmaLinux provisioning using config/; use deploy_management for updates."""
import json
from pathlib import Path
import secrets
import subprocess

from config_source import DEPLOYMENT, configure_signal, firewall_ports, read_config, render, template_values
from deploy_management import (BUDGET_UNIT, TURN_DEPENDENCY, RUNTIME_FILES, budget_rules,
                               configure_nginx, configure_turn, healthy)


def run(*args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def write(path, text, mode=0o644):
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text)
    target.chmod(mode)


def main():
    source = Path(__file__).resolve().parent
    run('/opt/codepet-venv/bin/python', '-m', 'unittest', 'discover', '-s', str(source / 'tests'), '-q', cwd=source)
    if subprocess.run(['id', 'codepet-signal'], capture_output=True).returncode:
        run('useradd', '--system', '--home-dir', '/var/lib/codepet-signal', '--shell', '/sbin/nologin', 'codepet-signal')
    path = Path('/etc/codepet-signal/config.json')
    existing = json.loads(path.read_text()) if path.exists() else {
        'turnSecret': secrets.token_urlsafe(48), 'publicIp': DEPLOYMENT['publicIp']}
    config = configure_signal(existing)
    write(path, json.dumps(config, indent=2) + '\n', 0o640)
    run('chown', 'root:codepet-signal', str(path))
    run('install', '-d', '-o', 'codepet-signal', '-g', 'codepet-signal', '-m', '700', '/var/lib/codepet-signal')
    run('install', '-d', '-o', 'coturn', '-g', 'coturn', '-m', '750', '/var/log/coturn')
    for name in RUNTIME_FILES:
        write(Path('/opt/codepet-signal', name), (source / name).read_text())
    for name in ('codepet-signal.service', 'codepet-cert-renew.service', 'codepet-cert-renew.timer'):
        write(Path('/etc/systemd/system', name), read_config(name))
    write('/etc/systemd/system/codepet-turn-budget.service', BUDGET_UNIT)
    write('/etc/systemd/system/coturn.service.d/codepet-budget.conf', TURN_DEPENDENCY)
    write('/etc/codepet-signal/turn-budget.nft', budget_rules())
    values = template_values(config['publicIp'], config['turnSecret'])
    nginx = Path('/etc/nginx/conf.d/codepet-signal.conf')
    write(nginx, configure_nginx(nginx.read_text()) if nginx.exists()
          else render('nginx.conf.tpl', **values))
    # Keep distro's default server off ACME's standalone port; application TLS
    # listeners and the separately managed VPN on 443 are not changed.
    nginx_main = Path('/etc/nginx/nginx.conf')
    text = nginx_main.read_text()
    if 'listen       80;' in text:
        backup = Path('/etc/nginx/nginx.conf.before-codepet')
        if not backup.exists():
            backup.write_text(text)
        nginx_main.write_text(text.replace('listen       80;', 'listen       127.0.0.1:8080;')
                              .replace('listen       [::]:80;', '# Default IPv6 HTTP listener disabled for standalone ACME.'))
    turn = Path('/etc/coturn/turnserver.conf')
    write(turn, configure_turn(turn.read_text()) if turn.exists()
          else render('turnserver.conf.tpl', **values), 0o640)
    run('chown', 'root:coturn', str(turn))
    write('/etc/logrotate.d/coturn', read_config('coturn.logrotate'))
    hook = '/etc/letsencrypt/renewal-hooks/deploy/codepet-reload.sh'
    write(hook, render('codepet-reload.sh.tpl', **values), 0o750)
    run(hook)
    for port in firewall_ports():
        run('firewall-cmd', '--permanent', '--add-port=' + port)
        run('firewall-cmd', '--add-port=' + port)
    run('nginx', '-t')
    run('nft', '--check', '-f', '/etc/codepet-signal/turn-budget.nft')
    run('logrotate', '--debug', '/etc/logrotate.conf')
    run('systemctl', 'daemon-reload')
    run('systemctl', 'enable', '--now', 'nginx', 'codepet-turn-budget', 'coturn', 'codepet-signal', 'codepet-cert-renew.timer')
    healthy()


if __name__ == '__main__':
    main()
