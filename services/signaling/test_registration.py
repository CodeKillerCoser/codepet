import base64
import json
import sqlite3
import tempfile
import time
import unittest
from aiohttp.test_utils import TestClient, TestServer
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from server import create_app, digest


class RegistrationTest(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = self.directory.name + '/state.db'
        self.key = Ed25519PrivateKey.generate()
        await self.start()

    async def start(self, enabled=True, capacity=1024):
        self.client = TestClient(TestServer(create_app(self.path, 'test-turn', '192.0.2.1', enabled, capacity)))
        await self.client.start_server()

    async def asyncTearDown(self):
        await self.client.close()
        self.directory.cleanup()

    def envelope(self, device='new-host', role='host', token='private-host-token', key=None, **changes):
        key = key or self.key
        payload = json.dumps(dict(v=1, kind='device.register', deviceId=device, role=role,
                                  tokenHash=digest(token), expires=int(time.time())+60, **changes)).encode()
        return {'payload': base64.b64encode(payload).decode(),
                'signature': base64.b64encode(key.sign(payload)).decode(),
                'publicKey': base64.b64encode(key.public_key().public_bytes_raw()).decode()}

    async def enroll(self, envelope=None, headers=None):
        return await self.client.post('/v1/devices/register', json=envelope or self.envelope(), headers=headers)

    async def test_restart_and_response_loss_are_idempotent_and_host_can_sync(self):
        envelope = self.envelope()
        for _ in range(2):
            response = await self.enroll(envelope)
            self.assertEqual(response.status, 200)
            self.assertEqual((await response.json())['deviceId'], 'new-host')
        await self.client.close()
        await self.start()
        self.assertEqual((await self.enroll(envelope)).status, 200)
        response = await self.client.put('/v1/clients', headers={'Authorization':'Bearer private-host-token'}, json={'clients':[]})
        self.assertEqual(response.status, 200)
        with sqlite3.connect(self.path) as db:
            self.assertEqual(db.execute('select count(*) from device_registrations').fetchone()[0], 1)

    async def test_key_role_and_token_cannot_be_replaced(self):
        await self.enroll()
        for envelope in [self.envelope(key=Ed25519PrivateKey.generate()),
                         self.envelope(token='different'), self.envelope(role='remote')]:
            self.assertEqual((await self.enroll(envelope)).status, 409)

    async def test_remote_enrollment_never_grants_host_or_turn_access(self):
        self.assertEqual((await self.enroll(self.envelope('remote-one', 'remote', 'remote-secret'))).status, 200)
        for method, path, body in [('GET','/v1/ice',None), ('PUT','/v1/clients',{'clients':[]}), ('GET','/v1/offers',None)]:
            response = await self.client.request(method, path, headers={'Authorization':'Bearer remote-secret'}, json=body)
            self.assertEqual(response.status, 401)

    async def test_legacy_host_requires_its_existing_authorization(self):
        with sqlite3.connect(self.path) as db:
            db.execute('insert into hosts values (?,?)', ('new-host', digest('private-host-token')))
        self.assertEqual((await self.enroll()).status, 409)
        self.assertEqual((await self.enroll(headers={'Authorization':'Bearer private-host-token'})).status, 200)

    async def test_invalid_signature_expired_or_malformed_registration_is_rejected(self):
        bad = self.envelope()
        bad['signature'] = base64.b64encode(bytes(64)).decode()
        expired_payload = json.loads(base64.b64decode(self.envelope()['payload']))
        expired_payload['expires'] = int(time.time()) - 1
        raw = json.dumps(expired_payload).encode()
        expired = dict(self.envelope(), payload=base64.b64encode(raw).decode(), signature=base64.b64encode(self.key.sign(raw)).decode())
        for envelope in [bad, expired, {'publicKey':'invalid'}, self.envelope(role='admin')]:
            self.assertEqual((await self.enroll(envelope)).status, 400)

    async def test_capacity_and_disabled_policy(self):
        await self.client.close()
        await self.start(capacity=1)
        self.assertEqual((await self.enroll()).status, 200)
        self.assertEqual((await self.enroll(self.envelope('another', token='another'))).status, 503)
        self.assertEqual((await self.enroll()).status, 200)
        await self.client.close()
        await self.start(enabled=False)
        self.assertEqual((await self.enroll()).status, 403)

    async def test_registration_rate_limit_and_logs_do_not_contain_secrets(self):
        envelope = self.envelope()
        with self.assertLogs('codepet.signal', level='INFO') as logs:
            for _ in range(30):
                self.assertEqual((await self.enroll(envelope)).status, 200)
            self.assertEqual((await self.enroll(envelope)).status, 429)
        output = str(logs.output)
        for value in [envelope['payload'], envelope['signature'], 'private-host-token', envelope['publicKey']]:
            self.assertNotIn(value, output)
        self.assertIn('/v1/devices/register', output)

if __name__ == '__main__':
    unittest.main()
