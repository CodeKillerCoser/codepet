"""Private CodePet rendezvous. Only opaque signed SDP, never Gateway payloads."""
import asyncio
import base64
import hashlib
import hmac
import json
import os
import re
import sqlite3
import time
from aiohttp import web

ID = re.compile(r"^[a-zA-Z0-9_-]{1,100}$")


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def create_app(db_path, turn_secret, public_ip):
    db = sqlite3.connect(db_path)
    db.executescript("""
      CREATE TABLE IF NOT EXISTS hosts (id TEXT PRIMARY KEY, token_hash TEXT UNIQUE NOT NULL);
      CREATE TABLE IF NOT EXISTS clients (host TEXT, id TEXT, token_hash TEXT UNIQUE NOT NULL,
        PRIMARY KEY(host,id));
    """)
    pending = {}
    rates = {}

    @web.middleware
    async def guard(request, handler):
        if request.path == '/health':
            return web.json_response({'status': 'ok', 'protocol': 'codepet-signal-v1'})
        auth = request.headers.get('Authorization', '')
        if not auth.startswith('Bearer ') or len(auth) > 256:
            raise web.HTTPUnauthorized()
        token = digest(auth[7:])
        row = db.execute('SELECT id FROM hosts WHERE token_hash=?', (token,)).fetchone()
        if row:
            request['actor'] = ('host', row[0], None)
        else:
            row = db.execute('SELECT host,id FROM clients WHERE token_hash=?', (token,)).fetchone()
            if not row:
                raise web.HTTPUnauthorized()
            request['actor'] = ('client', *row)
        now = time.monotonic()
        for key in list(rates):
            if rates[key][0] < now - 60:
                del rates[key]
        key = (token, 'turn' if request.path == '/v1/ice' else 'signal')
        start, count = rates.get(key, (now, 0))
        if count >= (12 if key[1] == 'turn' else 180):
            raise web.HTTPTooManyRequests()
        rates[key] = (start, count + 1)
        for key in list(pending):
            if pending[key]['deadline'] < now:
                del pending[key]
        response = await handler(request)
        response.headers['Cache-Control'] = 'no-store'
        return response

    def actor(request, role):
        kind, host, client = request['actor']
        if kind != role:
            raise web.HTTPForbidden()
        return host, client

    async def body(request):
        try:
            value = await request.json()
            if not isinstance(value, dict):
                raise ValueError()
            return value
        except (ValueError, TypeError):
            raise web.HTTPBadRequest()

    def identifier(value):
        if not isinstance(value, str) or not ID.fullmatch(value):
            raise web.HTTPBadRequest()
        return value

    def envelope(value):
        if (not isinstance(value, dict) or set(value) != {'payload', 'signature'}
                or not isinstance(value['payload'], str) or len(value['payload']) > 90000
                or not isinstance(value['signature'], str) or len(value['signature']) != 88):
            raise web.HTTPBadRequest()
        return value

    async def clients(request):
        host, _ = actor(request, 'host')
        entries = (await body(request)).get('clients')
        if not isinstance(entries, list) or len(entries) > 32:
            raise web.HTTPBadRequest()
        checked = []
        for item in entries:
            if not isinstance(item, dict):
                raise web.HTTPBadRequest()
            client = identifier(item.get('id'))
            hashed = item.get('tokenHash', '')
            if not isinstance(hashed, str) or not re.fullmatch('[0-9a-f]{64}', hashed):
                raise web.HTTPBadRequest()
            checked.append((host, client, hashed))
        try:
            with db:
                db.execute('DELETE FROM clients WHERE host=?', (host,))
                db.executemany('INSERT INTO clients VALUES (?,?,?)', checked)
        except sqlite3.IntegrityError:
            raise web.HTTPConflict()
        valid = {entry[1] for entry in checked}
        for key in list(pending):
            if key[0] == host and key[1] not in valid:
                del pending[key]
        return web.json_response({'ok': True})

    async def ice(request):
        _, host, client = request['actor']
        # 24h credentials; client transport must refresh by reconnecting before expiry.
        expiry = int(time.time()) + 86400
        username = f'{expiry}:{host}:{client or "host"}'
        password = base64.b64encode(hmac.new(turn_secret.encode(), username.encode(), hashlib.sha1).digest()).decode()
        return web.json_response({'expires': expiry, 'iceServers': [
            {'urls': [f'stun:{public_ip}:3478'], 'username': '', 'credential': ''},
            {'urls': [f'turn:{public_ip}:3478?transport=udp',
                      f'turn:{public_ip}:3478?transport=tcp',
                      f'turns:{public_ip}:5349?transport=tcp'],
             'username': username, 'credential': password}]})

    async def offer(request):
        host, client = actor(request, 'client')
        value = await body(request)
        attempt = identifier(value.get('attempt'))
        key = (host, client)
        previous = pending.get(key)
        if previous and previous['attempt'] == attempt:
            raise web.HTTPConflict()
        if len(pending) >= 128 and key not in pending:
            raise web.HTTPServiceUnavailable()
        pending[key] = {'attempt': attempt, 'envelope': envelope(value.get('envelope')),
                        'answer': None, 'delivered': False, 'deadline': time.monotonic() + 60}
        return web.json_response({'ok': True})

    async def offers(request):
        host, _ = actor(request, 'host')
        # Bounded polling, one offer per client; no unbounded tasks/mailboxes.
        result = []
        for (owner, client), value in pending.items():
            if owner == host and not value['delivered']:
                result.append({'client': client, 'attempt': value['attempt'], 'envelope': value['envelope']})
                value['delivered'] = True
        return web.json_response({'offers': result})

    async def answer(request):
        host, _ = actor(request, 'host')
        value = await body(request)
        key = (host, identifier(value.get('client')))
        entry = pending.get(key)
        if not entry or entry['attempt'] != value.get('attempt') or entry['answer'] is not None:
            raise web.HTTPConflict()
        entry['answer'] = envelope(value.get('envelope'))
        return web.json_response({'ok': True})

    async def get_answer(request):
        host, client = actor(request, 'client')
        entry = pending.get((host, client))
        if not entry or entry['attempt'] != request.query.get('attempt'):
            raise web.HTTPNotFound()
        return web.json_response({'answer': entry['answer']})

    app = web.Application(middlewares=[guard], client_max_size=100000)
    app.router.add_get('/health', lambda _: web.json_response({'status': 'ok'}))
    app.router.add_put('/v1/clients', clients)
    app.router.add_get('/v1/ice', ice)
    app.router.add_post('/v1/offers', offer)
    app.router.add_get('/v1/offers', offers)
    app.router.add_post('/v1/answers', answer)
    app.router.add_get('/v1/answer', get_answer)

    async def cleanup(_):
        db.close()
    app.on_cleanup.append(cleanup)
    return app


if __name__ == '__main__':
    config = json.load(open(os.environ.get('CODEPET_SIGNAL_CONFIG', '/etc/codepet-signal/config.json')))
    web.run_app(create_app(config['database'], config['turnSecret'], config['publicIp']),
                host='127.0.0.1', port=8787, access_log=None)
