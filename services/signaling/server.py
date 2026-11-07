"""Private CodePet rendezvous. Only opaque signed SDP, never Gateway payloads."""
import asyncio
import base64
import hashlib
import hmac
import json
import logging
import os
import re
import sqlite3
import time
import threading
from aiohttp import web
from registration import DeviceRegistration

LOG = logging.getLogger('codepet.signal')


def diagnostic(event, **fields):
    def clean(value):
        return re.sub(r'[\x00-\x1f\x7f]', ' ', str(value))
    LOG.info('%s%s', event, ''.join(f' | {key}={clean(value)}' for key, value in fields.items()), stacklevel=2)


ID = re.compile(r"^[a-zA-Z0-9_-]{1,100}$")


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def create_app(db_path, turn_secret, public_ip, registration_enabled=False, max_devices=1024):
    db = sqlite3.connect(db_path)
    db.executescript("""
      CREATE TABLE IF NOT EXISTS hosts (id TEXT PRIMARY KEY, token_hash TEXT UNIQUE NOT NULL);
      CREATE TABLE IF NOT EXISTS clients (host TEXT, id TEXT, token_hash TEXT UNIQUE NOT NULL,
        PRIMARY KEY(host,id));
    """)
    pending = {}
    trickle_hosts = set()
    invitations = {}
    rates = {}
    registration = DeviceRegistration(db, registration_enabled, max_devices)

    @web.middleware
    async def observe(request, handler):
        started = time.monotonic()
        status = 500
        try:
            response = await handler(request)
            status = response.status
            return response
        except web.HTTPException as error:
            status = error.status
            raise
        finally:
            # Never log query strings, Authorization, envelopes or exception text.
            # Successful empty polls are represented by actual mailbox stage events.
            quiet_success = request.path == '/v1/ice-updates' or (request.method == 'PUT' and request.path == '/v1/clients')
            if request.path != '/health' and (status != 200 or (request.method != 'GET' and not quiet_success) or request.path == '/v1/ice'):
                kind, host, client = request.get('actor', (None, None, None))
                diagnostic('http.complete', method=request.method,
                           route=request.path if request.path in {'/v1/ice','/v1/ice-updates','/v1/offers','/v1/answers','/v1/answer','/v1/clients','/v1/devices/register'} else 'unknown',
                           status=status, role=kind, host=host, client=client,
                           durationMs=round((time.monotonic()-started)*1000))

    @web.middleware
    async def guard(request, handler):
        if request.path == '/health':
            return web.json_response({'status': 'ok', 'protocol': 'codepet-signal-v1', 'deviceRegistration': registration_enabled,
                                      'features': ['client-trickle-ice-v1']})
        if request.path == '/v1/devices/register' and request.method == 'POST':
            return await handler(request)
        auth = request.headers.get('Authorization', '')
        if not auth.startswith('Bearer ') or len(auth) > 256:
            raise web.HTTPUnauthorized()
        token = digest(auth[7:])
        now = time.monotonic()
        for key in list(invitations):
            if invitations[key]['deadline'] <= now:
                del invitations[key]
        row = db.execute('SELECT id FROM hosts WHERE token_hash=?', (token,)).fetchone()
        if row:
            request['actor'] = ('host', row[0], None)
        else:
            row = db.execute('SELECT host,id FROM clients WHERE token_hash=?', (token,)).fetchone()
            if row:
                request['actor'] = ('client', *row)
            else:
                invitation = next((key for key, entry in invitations.items()
                                   if hmac.compare_digest(entry['tokenHash'], token)), None)
                if invitation is None or request.path != '/v1/invitation-exchange':
                    raise web.HTTPUnauthorized()
                request['actor'] = ('invite', *invitation)
        now = time.monotonic()
        for key in list(rates):
            if rates[key][0] < now - 60:
                del rates[key]
        category = 'trickle' if request.path == '/v1/ice-updates' else 'turn' if request.path == '/v1/ice' else 'signal'
        key = (token, category)
        start, count = rates.get(key, (now, 0))
        if count >= (12 if key[1] == 'turn' else 600 if key[1] == 'trickle' else 180):
            raise web.HTTPTooManyRequests()
        rates[key] = (start, count + 1)
        for key in list(pending):
            if pending[key]['deadline'] < now:
                diagnostic('mailbox.expired', host=key[0], client=key[1], attempt=pending[key]['attempt'], answered=pending[key]['answer'] is not None)
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
        value = await body(request)
        entries = value.get('clients')
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
        previous = db.execute('SELECT host,id,token_hash FROM clients WHERE host=?', (host,)).fetchall()
        changed = sorted(previous) != sorted(checked) or (host in trickle_hosts) != (value.get('trickleIce') is True)
        try:
            with db:
                db.execute('DELETE FROM clients WHERE host=?', (host,))
                db.executemany('INSERT INTO clients VALUES (?,?,?)', checked)
        except sqlite3.IntegrityError:
            raise web.HTTPConflict()
        valid = {entry[1] for entry in checked}
        if value.get('trickleIce') is True:
            trickle_hosts.add(host)
        else:
            trickle_hosts.discard(host)
        for key in list(pending):
            if key[0] == host and key[1] not in valid:
                del pending[key]
        if changed:
            diagnostic('clients.changed', host=host, count=len(checked), trickleIce=host in trickle_hosts)
        return web.json_response({'ok': True})

    async def ice(request):
        if request['actor'][0] not in ('host', 'client'):
            raise web.HTTPForbidden()
        _, host, client = request['actor']
        # 24h credentials; client transport must refresh by reconnecting before expiry.
        expiry = int(time.time()) + 86400
        username = f'{expiry}:{host}:{client or "host"}'
        password = base64.b64encode(hmac.new(turn_secret.encode(), username.encode(), hashlib.sha1).digest()).decode()
        return web.json_response({'expires': expiry, 'trickleIce': host in trickle_hosts, 'iceServers': [
            {'urls': [f'stun:{public_ip}:3478'], 'username': '', 'credential': ''},
            {'urls': [f'turn:{public_ip}:3478?transport=udp',
                      f'turn:{public_ip}:3478?transport=tcp',
                      f'turns:{public_ip}:5349?transport=tcp'],
             'username': username, 'credential': password}]})

    async def offer(request):
        host, client = actor(request, 'client')
        value = await body(request)
        attempt = identifier(value.get('attempt'))
        key = (host, client, attempt)
        if key in pending:
            raise web.HTTPConflict()
        if len(pending) >= 128 or sum(k[:2] == key[:2] for k in pending) >= 12:
            raise web.HTTPServiceUnavailable()
        pending[key] = {'attempt': attempt, 'envelope': envelope(value.get('envelope')),
                        'answer': None, 'delivered': False, 'updates': [], 'deadline': time.monotonic() + 60}
        diagnostic('offer.accepted', host=host, client=client, attempt=attempt, envelopeBytes=len(pending[key]['envelope']['payload']))
        return web.json_response({'ok': True})

    async def offers(request):
        host, _ = actor(request, 'host')
        try:
            limit = int(request.query.get('limit', '32'))
        except ValueError:
            raise web.HTTPBadRequest()
        if not 1 <= limit <= 32:
            raise web.HTTPBadRequest()
        # Attempts coexist; polling only consumes what the Host can admit.
        result = []
        for (owner, client, _), value in pending.items():
            if owner == host and not value['delivered']:
                result.append({'client': client, 'attempt': value['attempt'], 'envelope': value['envelope']})
                value['delivered'] = True
                diagnostic('offer.delivered', host=host, client=client, attempt=value['attempt'], ageMs=round((time.monotonic()-(value['deadline']-60))*1000))
                if len(result) >= limit:
                    break
        return web.json_response({'offers': result})

    async def answer(request):
        host, _ = actor(request, 'host')
        value = await body(request)
        key = (host, identifier(value.get('client')), identifier(value.get('attempt')))
        entry = pending.get(key)
        if not entry or entry['attempt'] != value.get('attempt') or entry['answer'] is not None:
            raise web.HTTPConflict()
        entry['answer'] = envelope(value.get('envelope'))
        diagnostic('answer.accepted', host=host, client=key[1], attempt=entry['attempt'], ageMs=round((time.monotonic()-(entry['deadline']-60))*1000))
        return web.json_response({'ok': True})

    async def get_answer(request):
        host, client = actor(request, 'client')
        entry = pending.get((host, client, identifier(request.query.get('attempt'))))
        if not entry or entry['attempt'] != request.query.get('attempt'):
            raise web.HTTPNotFound()
        if entry['answer'] is not None and not entry.get('answerLogged'):
            entry['answerLogged'] = True
            diagnostic('answer.delivered', host=host, client=client, attempt=entry['attempt'], ageMs=round((time.monotonic()-(entry['deadline']-60))*1000))
        return web.json_response({'answer': entry['answer']})

    async def cancel_offer(request):
        host, client = actor(request, 'client')
        pending.pop((host, client, identifier(request.query.get('attempt'))), None)
        return web.json_response({'ok': True})

    async def ice_update(request):
        host, client = actor(request, 'client')
        value = await body(request)
        key = (host, client, identifier(value.get('attempt')))
        entry = pending.get(key)
        if entry is None:
            raise web.HTTPNotFound()
        sequence = value.get('sequence')
        if type(sequence) is not int or not 0 <= sequence < 128:
            raise web.HTTPBadRequest()
        sealed = envelope(value.get('envelope'))
        if len(sealed['payload']) > 16000:
            raise web.HTTPRequestEntityTooLarge(max_size=16000, actual_size=len(sealed['payload']))
        updates = entry['updates']
        # Idempotent retries must not replace a previously accepted signed batch.
        if sequence < len(updates):
            if updates[sequence] != sealed:
                raise web.HTTPConflict()
        elif sequence == len(updates):
            if sum(len(v['payload']) for v in updates) + len(sealed['payload']) > 128 * 1024:
                raise web.HTTPRequestEntityTooLarge(max_size=128 * 1024, actual_size=128 * 1024 + 1)
            updates.append(sealed)
        else:
            raise web.HTTPConflict()
        return web.json_response({'ok': True})

    async def ice_updates(request):
        host, _ = actor(request, 'host')
        key = (host, identifier(request.query.get('client')), identifier(request.query.get('attempt')))
        entry = pending.get(key)
        if entry is None:
            raise web.HTTPNotFound()
        try:
            after = int(request.query.get('after', '0'))
        except ValueError:
            raise web.HTTPBadRequest()
        if not 0 <= after <= len(entry['updates']):
            raise web.HTTPBadRequest()
        return web.json_response({'updates': entry['updates'][after:after + 4]})

    async def publish_invitation(request):
        host, _ = actor(request, 'host')
        value = await body(request)
        key = (host, identifier(value.get('id')))
        token_hash = value.get('tokenHash')
        expires = value.get('expires')
        if (not isinstance(token_hash, str) or not re.fullmatch('[0-9a-f]{64}', token_hash)
                or type(expires) is not int or not int(time.time()) < expires <= int(time.time()) + 330):
            raise web.HTTPBadRequest()
        previous = invitations.get(key)
        if previous:
            if previous['tokenHash'] != token_hash or previous['expires'] != expires:
                raise web.HTTPConflict()
        else:
            if len(invitations) >= 128 or sum(k[0] == host for k in invitations) >= 4:
                raise web.HTTPServiceUnavailable()
            invitations[key] = {'tokenHash': token_hash, 'expires': expires,
                                'deadline': time.monotonic() + expires - time.time(),
                                'request': None, 'result': None}
        return web.json_response({'ok': True})

    def sealed(value):
        if not isinstance(value, str) or not 40 <= len(value) <= 16000:
            raise web.HTTPBadRequest()
        try:
            base64.b64decode(value, validate=True)
        except ValueError:
            raise web.HTTPBadRequest()
        return value

    async def invitation_exchange(request):
        key = actor(request, 'invite')
        entry = invitations[key]
        value = await body(request)
        message = {'requestId': identifier(value.get('requestId')), 'sealed': sealed(value.get('sealed'))}
        if entry['request'] is not None and entry['request'] != message:
            raise web.HTTPConflict()
        entry['request'] = message
        return web.json_response({'result': entry['result']})

    async def invitation_requests(request):
        host, _ = actor(request, 'host')
        # Redeliver until terminal result: losing an HTTP response must not strand a request.
        return web.json_response({'requests': [dict(id=key[1], **entry['request'])
            for key, entry in invitations.items() if key[0] == host
            and entry['request'] is not None and entry['result'] is None]})

    async def invitation_result(request):
        host, _ = actor(request, 'host')
        value = await body(request)
        entry = invitations.get((host, identifier(value.get('id'))))
        if entry is None or entry['request'] is None or entry['request']['requestId'] != value.get('requestId'):
            raise web.HTTPConflict()
        result = sealed(value.get('result'))
        if entry['result'] is not None and entry['result'] != result:
            raise web.HTTPConflict()
        entry['result'] = result
        return web.json_response({'ok': True})

    app = web.Application(middlewares=[observe, guard], client_max_size=100000)
    app.router.add_post('/v1/devices/register', registration.register)
    app.router.add_put('/v1/invitations', publish_invitation)
    app.router.add_post('/v1/invitation-exchange', invitation_exchange)
    app.router.add_get('/v1/invitation-requests', invitation_requests)
    app.router.add_post('/v1/invitation-results', invitation_result)
    app.router.add_get('/health', lambda _: web.json_response({'status': 'ok'}))
    app.router.add_put('/v1/clients', clients)
    app.router.add_get('/v1/ice', ice)
    app.router.add_post('/v1/offers', offer)
    app.router.add_post('/v1/ice-updates', ice_update)
    app.router.add_get('/v1/ice-updates', ice_updates)
    app.router.add_delete('/v1/offers', cancel_offer)
    app.router.add_get('/v1/offers', offers)
    app.router.add_post('/v1/answers', answer)
    app.router.add_get('/v1/answer', get_answer)

    async def cleanup(_):
        db.close()
    app.on_cleanup.append(cleanup)
    return app


if __name__ == '__main__':
    factory = logging.getLogRecordFactory()
    def record_factory(*args, **kwargs):
        record = factory(*args, **kwargs)
        record.thread = threading.get_native_id()
        return record
    logging.setLogRecordFactory(record_factory)
    logging.Formatter.converter = time.gmtime
    logging.basicConfig(level=logging.INFO,
        format='%(asctime)s.%(msecs)03d+00:00 pid=%(process)d thread=%(thread)d %(levelname)s %(pathname)s:%(lineno)d [%(name)s] %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S')
    config = json.load(open(os.environ.get('CODEPET_SIGNAL_CONFIG', '/etc/codepet-signal/config.json')))
    web.run_app(create_app(config['database'], config['turnSecret'], config['publicIp'],
                           config.get('deviceRegistration', False), config.get('maxDevices', 1024)),
                host='127.0.0.1', port=8787, access_log=None)
