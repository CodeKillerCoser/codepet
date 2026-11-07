"""Device-key enrollment. Account policy can be added here independently of rendezvous."""
import base64
import hashlib
import hmac
import json
import re
import sqlite3
import time

from aiohttp import web
from policy import Limits, RateLimiter, client_source, unavailable
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey


class DeviceRegistration:
    def __init__(self, db, enabled=False, max_devices=1024, limits=None, trusted_proxies=()):
        self.db, self.enabled, self.max_devices = db, enabled, max_devices
        self.limits = limits or Limits()
        self.rates = RateLimiter(self.limits.rate_buckets)
        self.trusted_proxies = trusted_proxies
        db.execute('''CREATE TABLE IF NOT EXISTS device_registrations (
            id TEXT PRIMARY KEY, role TEXT NOT NULL, public_key TEXT NOT NULL,
            token_hash TEXT UNIQUE NOT NULL, created_at INTEGER NOT NULL)''')

    async def register(self, request):
        if not self.enabled:
            raise web.HTTPForbidden(text='device_registration_disabled')
        self.rates.check(client_source(request, self.trusted_proxies), self.limits.registration_per_ip_minute)
        try:
            envelope = await request.json()
            if not isinstance(envelope, dict) or set(envelope) != {'payload', 'signature', 'publicKey'}:
                raise ValueError()
            if any(not isinstance(envelope[k], str) for k in envelope):
                raise ValueError()
            if len(envelope['payload']) > 4096 or len(envelope['signature']) != 88 or len(envelope['publicKey']) != 44:
                raise ValueError()
            payload = base64.b64decode(envelope['payload'], validate=True)
            public = base64.b64decode(envelope['publicKey'], validate=True)
            Ed25519PublicKey.from_public_bytes(public).verify(
                base64.b64decode(envelope['signature'], validate=True), payload)
            body = json.loads(payload)
            if not isinstance(body, dict) or set(body) != {'v', 'kind', 'deviceId', 'role', 'tokenHash', 'expires'}:
                raise ValueError()
            if body['v'] != 1 or body['kind'] != 'device.register' or body['role'] not in ('host', 'remote'):
                raise ValueError()
            if not isinstance(body['deviceId'], str) or not re.fullmatch(r'[a-zA-Z0-9_-]{1,100}', body['deviceId']):
                raise ValueError()
            if not isinstance(body['tokenHash'], str) or not re.fullmatch(r'[0-9a-f]{64}', body['tokenHash']):
                raise ValueError()
            if type(body['expires']) is not int or not time.time() < body['expires'] <= time.time() + 120:
                raise ValueError()
        except (ValueError, TypeError, KeyError, InvalidSignature):
            raise web.HTTPBadRequest(text='invalid_device_registration')
        device, role, token_hash = body['deviceId'], body['role'], body['tokenHash']
        public_key = base64.b64encode(public).decode()
        existing = self.db.execute('SELECT role,public_key,token_hash FROM device_registrations WHERE id=?', (device,)).fetchone()
        if existing:
            # Replays are harmless and retries after response loss are idempotent.
            # A registration cannot silently rotate an existing key or bearer.
            if existing != (role, public_key, token_hash):
                raise web.HTTPConflict(text='device_identity_conflict')
        else:
            legacy = self.db.execute('SELECT token_hash FROM hosts WHERE id=?', (device,)).fetchone()
            if legacy:
                auth = request.headers.get('Authorization', '')
                supplied = hashlib.sha256(auth[7:].encode()).hexdigest() if auth.startswith('Bearer ') else ''
                if role != 'host' or not hmac.compare_digest(legacy[0], supplied) or legacy[0] != token_hash:
                    raise web.HTTPConflict(text='device_identity_conflict')
            if self.db.execute('SELECT count(*) FROM device_registrations').fetchone()[0] >= self.max_devices:
                raise unavailable('device_registration_capacity')
            try:
                with self.db:
                    self.db.execute('INSERT INTO device_registrations VALUES (?,?,?,?,?)',
                                    (device, role, public_key, token_hash, int(time.time())))
                    if role == 'host' and not legacy:
                        self.db.execute('INSERT INTO hosts VALUES (?,?)', (device, token_hash))
            except sqlite3.IntegrityError:
                raise web.HTTPConflict(text='device_identity_conflict')
        request['actor'] = (role, device, None)
        return web.json_response({'protocol': 'codepet-device-registration-v1',
                                  'deviceId': device, 'role': role, 'status': 'registered'},
                                 headers={'Cache-Control': 'no-store'})
