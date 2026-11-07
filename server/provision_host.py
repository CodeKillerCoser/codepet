"""Admin-only provisioning. Writes credentials to a mode-600 file, never stdout."""
import base64
import hashlib
import json
import os
import pathlib
import re
import secrets
import sqlite3
import sys

host, output = sys.argv[1:]
if not re.fullmatch(r'[a-zA-Z0-9_-]{1,100}', host):
    raise SystemExit('Invalid Host ID')
path = pathlib.Path(output)
if path.exists():
    raise SystemExit('Refusing to replace an existing Host configuration')
config = json.load(open('/etc/codepet-signal/config.json'))
token = secrets.token_urlsafe(32)
state = {'serviceUrl': f"https://{config['publicIp']}:8443", 'hostToken': token,
         'seed': base64.b64encode(secrets.token_bytes(32)).decode(), 'peers': {}}
db = sqlite3.connect(config['database'])
with db:
    db.execute('INSERT INTO hosts VALUES (?,?)', (host, hashlib.sha256(token.encode()).hexdigest()))
with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as file:
    json.dump(state, file)
print('Host provisioned; configuration written to private file.')
