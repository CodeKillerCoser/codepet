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

    async def test_diagnostics_correlate_mailbox_without_logging_secrets(self):
        import json
        secret_payload = base64.b64encode(b'private SDP and ICE password').decode()
        with self.assertLogs('codepet.signal', level='INFO') as captured:
            await self.call('PUT', '/v1/clients', 'host-token', {'clients': [{'id': 'client-a', 'tokenHash': digest('client-token')}]})
            envelope = {'payload': secret_payload, 'signature': base64.b64encode(bytes(64)).decode()}
            await self.call('POST', '/v1/offers', 'client-token', {'attempt': 'diagnostic-attempt', 'envelope': envelope})
            await self.call('GET', '/v1/offers', 'host-token')
            await self.call('POST', '/v1/answers', 'host-token', {'client': 'client-a', 'attempt': 'diagnostic-attempt', 'envelope': envelope})
            await self.call('GET', '/v1/answer?attempt=diagnostic-attempt', 'client-token')
            await self.call('GET', '/v1/answer?attempt=secret-query', 'invalid-secret-token')
        entries = [json.loads(record.getMessage()) for record in captured.records]
        for stage in ['offer.accepted','offer.delivered','answer.accepted','answer.delivered']:
            event = next(e for e in entries if e['event'] == stage)
            self.assertEqual(event['attempt'], 'diagnostic-attempt')
        text = str(entries)
        for secret in [secret_payload, envelope['signature'], 'host-token','client-token','invalid-secret-token','secret-query']:
            self.assertNotIn(secret, text)
        self.assertTrue(any(e.get('status') == 401 for e in entries))

    async def test_oversized_and_turn_limits(self):
        self.assertEqual((await self.call('PUT', '/v1/clients', 'host-token', {'clients': [None]})).status, 400)
        for _ in range(12):
            self.assertEqual((await self.call('GET', '/v1/ice', 'host-token')).status, 200)
        self.assertEqual((await self.call('GET', '/v1/ice', 'host-token')).status, 429)

    async def test_invitation_scope_retry_and_immutable_result(self):
        import time
        invite = {'id': 'invite-a', 'tokenHash': digest('invite-token'), 'expires': int(time.time()) + 120}
        self.assertEqual((await self.call('PUT', '/v1/invitations', 'host-token', invite)).status, 200)
        self.assertEqual((await self.call('PUT', '/v1/invitations', 'host-token', invite)).status, 200)
        self.assertEqual((await self.call('GET', '/v1/ice', 'invite-token')).status, 401)
        self.assertEqual((await self.call('POST', '/v1/offers', 'invite-token', {})).status, 401)
        message = {'requestId': 'request-a', 'sealed': base64.b64encode(bytes(48)).decode()}
        for _ in range(2):
            response = await self.call('POST', '/v1/invitation-exchange', 'invite-token', message)
            self.assertEqual(response.status, 200)
            self.assertIsNone((await response.json())['result'])
        for change in [dict(message, requestId='other'), dict(message, sealed=base64.b64encode(bytes(49)).decode())]:
            self.assertEqual((await self.call('POST', '/v1/invitation-exchange', 'invite-token', change)).status, 409)
        for _ in range(2):
            self.assertEqual(len((await (await self.call('GET', '/v1/invitation-requests', 'host-token')).json())['requests']), 1)
        self.assertEqual((await (await self.call('GET', '/v1/invitation-requests', 'other-host')).json())['requests'], [])
        result = {'id': 'invite-a', 'requestId': 'request-a', 'result': base64.b64encode(bytes(64)).decode()}
        self.assertEqual((await self.call('POST', '/v1/invitation-results', 'other-host', result)).status, 409)
        for _ in range(2):
            self.assertEqual((await self.call('POST', '/v1/invitation-results', 'host-token', result)).status, 200)
        self.assertEqual((await self.call('POST', '/v1/invitation-results', 'host-token', dict(result, result=message['sealed']))).status, 409)
        response = await self.call('POST', '/v1/invitation-exchange', 'invite-token', message)
        self.assertEqual((await response.json())['result'], result['result'])
        self.assertEqual(response.headers['Cache-Control'], 'no-store')

    async def test_invitation_expiry_and_invalid_publish(self):
        import time
        for expires in [True, int(time.time()) - 1, int(time.time()) + 3600]:
            self.assertEqual((await self.call('PUT', '/v1/invitations', 'host-token',
                {'id': 'invite', 'tokenHash': digest('secret'), 'expires': expires})).status, 400)


if __name__ == '__main__':
    unittest.main()
