import base64
import sqlite3
import tempfile
import unittest
from aiohttp.test_utils import TestClient, TestServer
from server import create_app, digest


class AdmissionTest(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory()
        path = self.directory.name + '/state.db'
        app = create_app(path, 'test-turn-secret', '192.0.2.1')
        with sqlite3.connect(path) as db:
            db.executemany('INSERT INTO hosts VALUES (?,?)', [('host-a', digest('host-token')), ('host-b', digest('other-host'))])
        self.client = TestClient(TestServer(app))
        await self.client.start_server()

    async def asyncTearDown(self):
        await self.client.close()
        self.directory.cleanup()

    async def call(self, method, path, token, body=None):
        return await self.client.request(method, path, headers={'Authorization': 'Bearer '+token}, json=body)

    async def test_scopes_revocation_and_mailbox(self):
        self.assertEqual((await self.call('GET', '/v1/ice', 'unknown')).status, 401)
        body = {'clients': [{'id': 'client-a', 'tokenHash': digest('client-token')}]}
        self.assertEqual((await self.call('PUT', '/v1/clients', 'host-token', body)).status, 200)
        self.assertEqual((await self.call('PUT', '/v1/clients', 'client-token', body)).status, 403)
        envelope = {'payload': base64.b64encode(b'opaque').decode(), 'signature': base64.b64encode(bytes(64)).decode()}
        offer = {'attempt': 'attempt-a', 'envelope': envelope}
        self.assertEqual((await self.call('POST', '/v1/offers', 'client-token', offer)).status, 200)
        self.assertEqual((await self.call('POST', '/v1/offers', 'client-token', offer)).status, 409)
        self.assertEqual((await (await self.call('GET', '/v1/offers', 'other-host')).json())['offers'], [])
        self.assertEqual(len((await (await self.call('GET', '/v1/offers', 'host-token')).json())['offers']), 1)
        answer = {'client': 'client-a', 'attempt': 'attempt-a', 'envelope': envelope}
        self.assertEqual((await self.call('POST', '/v1/answers', 'other-host', answer)).status, 409)
        self.assertEqual((await self.call('POST', '/v1/answers', 'host-token', answer)).status, 200)
        self.assertEqual((await self.call('POST', '/v1/answers', 'host-token', answer)).status, 409)
        result = await (await self.call('GET', '/v1/answer?attempt=attempt-a', 'client-token')).json()
        self.assertEqual(result['answer'], envelope)
        self.assertEqual((await self.call('PUT', '/v1/clients', 'host-token', {'clients': []})).status, 200)
        self.assertEqual((await self.call('GET', '/v1/ice', 'client-token')).status, 401)

    async def test_oversized_and_turn_limits(self):
        self.assertEqual((await self.call('PUT', '/v1/clients', 'host-token', {'clients': [None]})).status, 400)
        for _ in range(12):
            self.assertEqual((await self.call('GET', '/v1/ice', 'host-token')).status, 200)
        self.assertEqual((await self.call('GET', '/v1/ice', 'host-token')).status, 429)


if __name__ == '__main__':
    unittest.main()
