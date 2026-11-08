import base64
import sqlite3
import tempfile
import unittest
from types import SimpleNamespace
import ipaddress

from aiohttp import web
from aiohttp.test_utils import TestClient, TestServer
from policy import Limits, RateLimiter, ByteBudget, Mailboxes, client_source
from server import create_app, digest
import test_registration


class PolicyTest(unittest.TestCase):
    def test_limits_reject_typos_and_invalid_bounds(self):
        for values in ({'typo': 1}, {'pending_total': 0}, {'pending_total': True}):
            with self.assertRaises(ValueError):
                Limits.parse(values)

    def test_only_trusted_proxy_can_supply_single_source_address(self):
        trusted = (ipaddress.ip_network('127.0.0.1/32'),)
        request = SimpleNamespace(remote='198.51.100.1', headers={'X-Real-IP': '192.0.2.2'})
        self.assertEqual(client_source(request, trusted), '198.51.100.1')
        request.remote = '127.0.0.1'
        self.assertEqual(client_source(request, trusted), '192.0.2.2')
        request.headers['X-Real-IP'] = '192.0.2.2, 198.51.100.1'
        with self.assertRaises(web.HTTPBadRequest):
            client_source(request, trusted)

    def test_rate_expiry_retry_and_other_identity_isolation(self):
        now = [0]
        limiter = RateLimiter(2, lambda: now[0])
        limiter.check('a', 1)
        now[0] = 10
        with self.assertRaises(web.HTTPTooManyRequests) as caught:
            limiter.check('a', 1)
        self.assertEqual(caught.exception.headers['Retry-After'], '50')
        limiter.check('b', 1)
        with self.assertRaises(web.HTTPServiceUnavailable):
            limiter.check('c', 1)
        now[0] = 61
        limiter.expire()
        limiter.check('c', 1)

    def test_shared_bytes_atomic_update_scope_quota_and_expiry(self):
        now = [0]
        budget = ByteBudget(10)
        offers = Mailboxes(3, 1, lambda k: k[0], budget, lambda: now[0])
        invites = Mailboxes(3, 1, lambda k: k[0], budget, lambda: now[0])
        offers['a', 1] = {'deadline': 2, 'payload': '1234'}
        invites['b', 1] = {'deadline': 10, 'payload': '1234'}
        with self.assertRaises(web.HTTPServiceUnavailable):
            offers.update_entry(('a', 1), payload='1234567')
        self.assertEqual(offers['a', 1]['payload'], '1234')
        self.assertEqual(budget.used, 8)
        with self.assertRaises(web.HTTPServiceUnavailable):
            offers['a', 2] = {'deadline': 2}
        now[0] = 3
        offers.expire()
        self.assertEqual(budget.used, 4)
        offers['a', 2] = {'deadline': 20, 'payload': '123456'}
        invites.pop(('b', 1))
        offers.pop(('a', 2))
        self.assertEqual(budget.used, 0)
        self.assertFalse(offers.counts)


class ProxyRegistrationTest(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = test_registration.RegistrationTest.asyncSetUp
    asyncTearDown = test_registration.RegistrationTest.asyncTearDown
    start = test_registration.RegistrationTest.start
    envelope = test_registration.RegistrationTest.envelope
    enroll = test_registration.RegistrationTest.enroll

    async def test_many_devices_share_proxy_but_keep_source_limits(self):
        await self.client.close()
        app = create_app(self.path, 'test-turn', '192.0.2.1', True,
                         limits={'registration_per_ip_minute': 2}, trusted_proxies=['127.0.0.1/32'])
        self.client = TestClient(TestServer(app))
        await self.client.start_server()
        for n in range(40):
            response = await self.enroll(self.envelope(f'phone-{n}', 'remote', f'token-{n}'),
                                        {'X-Real-IP': f'192.0.2.{n + 1}'})
            self.assertEqual(response.status, 200)
        envelope = self.envelope('phone-0', 'remote', 'token-0')
        self.assertEqual((await self.enroll(envelope, {'X-Real-IP': '192.0.2.1'})).status, 200)
        blocked = await self.enroll(envelope, {'X-Real-IP': '192.0.2.1'})
        self.assertEqual(blocked.status, 429)
        self.assertIn('Retry-After', blocked.headers)


class CapacityIntegrationTest(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = self.directory.name + '/state.db'
        self.client = None

    async def start(self, **limits):
        app = create_app(self.path, 'turn-test', '192.0.2.1', limits=limits)
        with sqlite3.connect(self.path) as db:
            db.execute('INSERT OR IGNORE INTO hosts VALUES (?,?)', ('host', digest('host-token')))
            db.executemany('INSERT OR IGNORE INTO clients VALUES (?,?,?)',
                           [('host', f'client-{n}', digest(f'token-{n}')) for n in range(160)])
        self.client = TestClient(TestServer(app))
        await self.client.start_server()

    async def asyncTearDown(self):
        if self.client:
            await self.client.close()
        self.directory.cleanup()

    async def call(self, method, path, token='token-0', body=None):
        return await self.client.request(method, path, headers={'Authorization': 'Bearer ' + token}, json=body)

    def offer(self, attempt, size=10):
        return {'attempt': attempt, 'envelope': {'payload': base64.b64encode(b'x' * size).decode(),
                                                'signature': base64.b64encode(bytes(64)).decode()}}

    async def test_480_parallel_mailboxes_not_capped_at_128(self):
        await self.start()
        # Retain every handshake concurrently; this tests admission, not network throughput.
        for n in range(160):
            for path in ('ipv6', 'ipv4', 'relay'):
                self.assertEqual((await self.call('POST', '/v1/offers', f'token-{n}', self.offer(path))).status, 200)
        for n in range(160):
            for path in ('ipv6', 'ipv4', 'relay'):
                self.assertEqual((await self.call('DELETE', f'/v1/offers?attempt={path}', f'token-{n}')).status, 200)

    async def test_mailbox_memory_exhaustion_release_and_retry(self):
        await self.start(mailbox_bytes=300)
        self.assertEqual((await self.call('POST', '/v1/offers', body=self.offer('a', 100))).status, 200)
        rejected = await self.call('POST', '/v1/offers', body=self.offer('b', 100))
        self.assertEqual(rejected.status, 503)
        self.assertEqual(await rejected.text(), 'mailbox_bytes_capacity')
        self.assertIn('Retry-After', rejected.headers)
        await self.call('DELETE', '/v1/offers?attempt=a')
        self.assertEqual((await self.call('POST', '/v1/offers', body=self.offer('b', 100))).status, 200)

    async def test_expired_global_slot_reclaimed_without_request_sweeps(self):
        import asyncio
        await self.start(pending_total=1, pending_ttl_seconds=1, cleanup_interval_seconds=1)
        self.assertEqual((await self.call('POST', '/v1/offers', body=self.offer('a'))).status, 200)
        await asyncio.sleep(2.05)
        self.assertEqual((await self.call('POST', '/v1/offers', body=self.offer('b'))).status, 200)
        self.assertEqual((await self.call('GET', '/v1/answer?attempt=a')).status, 404)

    async def test_unchanged_client_sync_does_not_commit_database_writes(self):
        await self.start()
        clients = {'clients': [{'id': 'client-0', 'tokenHash': digest('token-0')}]}
        await self.call('PUT', '/v1/clients', 'host-token', clients)
        with sqlite3.connect(self.path) as observer:
            before = observer.execute('PRAGMA data_version').fetchone()[0]
            for _ in range(3):
                await self.call('PUT', '/v1/clients', 'host-token', clients)
            self.assertEqual(observer.execute('PRAGMA data_version').fetchone()[0], before)
            await self.call('PUT', '/v1/clients', 'host-token', {'clients': []})
            self.assertGreater(observer.execute('PRAGMA data_version').fetchone()[0], before)
