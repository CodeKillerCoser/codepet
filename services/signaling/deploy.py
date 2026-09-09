"""Run as root on the authorized AlmaLinux VPS after installing prerequisites.
Does not touch the existing service on TCP 443. Secrets remain on the server.
"""
import json
import os
import pathlib
import secrets
import subprocess

def run(*args):
    subprocess.run(args, check=True)

def write(path, text, mode=0o644):
    target = pathlib.Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text)
    target.chmod(mode)

if subprocess.run(['id', 'codepet-signal'], capture_output=True).returncode:
    run('useradd', '--system', '--home-dir', '/var/lib/codepet-signal', '--shell', '/sbin/nologin', 'codepet-signal')
config_path = pathlib.Path('/etc/codepet-signal/config.json')
if not config_path.exists():
    write(str(config_path), json.dumps({'database': '/var/lib/codepet-signal/signal.db', 'turnSecret': secrets.token_urlsafe(48), 'publicIp': '172.96.254.12'}), 0o640)
run('chown', 'root:codepet-signal', str(config_path))
run('install', '-d', '-o', 'codepet-signal', '-g', 'codepet-signal', '-m', '700', '/var/lib/codepet-signal')
config = json.loads(config_path.read_text())
ip = config['publicIp']
write('/etc/systemd/system/codepet-signal.service', '''[Unit]
Description=CodePet authenticated WebRTC signaling
After=network-online.target
[Service]
User=codepet-signal
Group=codepet-signal
ExecStart=/opt/codepet-venv/bin/python /opt/codepet-signal/server.py
Restart=on-failure
RestartSec=3
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/codepet-signal
MemoryMax=192M
TasksMax=64
[Install]
WantedBy=multi-user.target
''')
write('/etc/nginx/conf.d/codepet-signal.conf', f'''limit_req_zone $binary_remote_addr zone=codepet_ip:10m rate=10r/s;
server {{
 listen 8443 ssl;
 server_name {ip};
 ssl_certificate /etc/letsencrypt/live/{ip}/fullchain.pem;
 ssl_certificate_key /etc/letsencrypt/live/{ip}/privkey.pem;
 ssl_protocols TLSv1.2 TLSv1.3;
 client_max_body_size 100k;
 client_body_timeout 10s;
 access_log off;
 location / {{
  limit_req zone=codepet_ip burst=30 nodelay;
  proxy_pass http://127.0.0.1:8787;
  proxy_read_timeout 20s;
  proxy_set_header Host $host;
 }}
}}
''')
# Standalone ACME owns port 80 briefly. The default nginx server must not bind it.
nginx = pathlib.Path('/etc/nginx/nginx.conf')
text = nginx.read_text()
if 'listen       80;' in text:
    backup = pathlib.Path('/etc/nginx/nginx.conf.before-codepet')
    if not backup.exists():
        backup.write_text(text)
    text = text.replace('listen       80;', 'listen       127.0.0.1:8080;').replace('listen       [::]:80;', '# Default IPv6 HTTP listener disabled for standalone ACME.')
    nginx.write_text(text)

write('/etc/coturn/turnserver.conf', f'''listening-port=3478
tls-listening-port=5349
listening-ip={ip}
relay-ip={ip}
min-port=49160
max-port=49259
fingerprint
use-auth-secret
static-auth-secret={config['turnSecret']}
realm=codepet
server-name=codepet
cert=/etc/coturn/codepet-fullchain.pem
pkey=/etc/coturn/codepet-privkey.pem
no-cli
no-multicast-peers
no-dtls
no-tcp-relay
stale-nonce=600
user-quota=8
total-quota=64
max-bps=2000000
bps-capacity=10000000
no-rfc5780
no-software-attribute
denied-peer-ip=0.0.0.0-0.255.255.255
denied-peer-ip=10.0.0.0-10.255.255.255
denied-peer-ip=100.64.0.0-100.127.255.255
denied-peer-ip=127.0.0.0-127.255.255.255
denied-peer-ip=169.254.0.0-169.254.255.255
denied-peer-ip=172.16.0.0-172.31.255.255
denied-peer-ip=192.168.0.0-192.168.255.255
denied-peer-ip=224.0.0.0-255.255.255.255
denied-peer-ip=::1
denied-peer-ip=fc00::-fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff
denied-peer-ip=fe80::-febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff
''', 0o640)
run('chown', 'root:coturn', '/etc/coturn/turnserver.conf')
write('/etc/letsencrypt/renewal-hooks/deploy/codepet-reload.sh', f'''#!/bin/sh
set -eu
install -o root -g coturn -m 640 /etc/letsencrypt/live/{ip}/fullchain.pem /etc/coturn/codepet-fullchain.pem
install -o root -g coturn -m 640 /etc/letsencrypt/live/{ip}/privkey.pem /etc/coturn/codepet-privkey.pem
systemctl reload nginx || true
systemctl reload coturn || true
''', 0o750)
run('/etc/letsencrypt/renewal-hooks/deploy/codepet-reload.sh')
write('/etc/systemd/system/codepet-cert-renew.service', '''[Unit]
Description=Renew CodePet short-lived IP certificate
[Service]
Type=oneshot
ExecStart=/opt/codepet-venv/bin/certbot renew --quiet
''')
write('/etc/systemd/system/codepet-cert-renew.timer', '''[Unit]
Description=Check CodePet IP certificate twice daily
[Timer]
OnCalendar=*-*-* 00,12:00:00
RandomizedDelaySec=1800
Persistent=true
[Install]
WantedBy=timers.target
''')
for port in ['8443/tcp', '3478/udp', '3478/tcp', '5349/tcp', '49160-49259/udp']:
    run('firewall-cmd', '--permanent', '--add-port='+port)
    run('firewall-cmd', '--add-port='+port)
run('nginx', '-t')
run('systemctl', 'daemon-reload')
run('systemctl', 'enable', '--now', 'nginx', 'coturn', 'codepet-signal', 'codepet-cert-renew.timer')
run('systemctl', 'restart', 'codepet-signal')
