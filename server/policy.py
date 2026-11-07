"""Bounded admission for the existing HTTP rendezvous; no business/device metadata."""
from collections import Counter
from dataclasses import dataclass, fields
import ipaddress
import math
import time

from aiohttp import web


@dataclass(frozen=True)
class Limits:
    pending_total: int = 2048
    pending_per_pair: int = 12
    pending_ttl_seconds: int = 60
    invitations_total: int = 1024
    invitations_per_host: int = 4
    mailbox_bytes: int = 32 * 1024 * 1024
    signal_per_minute: int = 600
    turn_per_minute: int = 120
    trickle_per_minute: int = 6000
    registration_per_ip_minute: int = 300
    rate_buckets: int = 32768
    cleanup_interval_seconds: int = 5

    @classmethod
    def parse(cls, value=None):
        value = {} if value is None else value
        if not isinstance(value, dict) or set(value) - {f.name for f in fields(cls)}:
            raise ValueError('Unknown signaling limit')
        if any(type(v) is not int or not 0 < v <= 2**31 - 1 for v in value.values()):
            raise ValueError('Signaling limits must be positive integers')
        return cls(**value)


def unavailable(reason):
    return web.HTTPServiceUnavailable(text=reason, headers={'Retry-After': '5'})


class RateLimiter:
    def __init__(self, capacity, clock=time.monotonic):
        self.capacity, self.clock, self.windows = capacity, clock, {}

    def expire(self):
        now = self.clock()
        for key, (start, _) in list(self.windows.items()):
            if start + 60 <= now:
                del self.windows[key]

    def check(self, key, limit):
        now = self.clock()
        start, count = self.windows.get(key, (now, 0))
        if start + 60 <= now:
            start, count = now, 0
        if count >= limit:
            raise web.HTTPTooManyRequests(text='rate_limited', headers={
                'Retry-After': str(max(1, math.ceil(start + 60 - now)))})
        if key not in self.windows and len(self.windows) >= self.capacity:
            raise unavailable('rate_bucket_capacity')
        self.windows[key] = (start, count + 1)


def client_source(request, trusted_proxies):
    """nginx overwrites X-Real-IP. Never trust headers from arbitrary peers."""
    source = request.remote or 'unknown'
    try:
        peer = ipaddress.ip_address(source)
    except ValueError:
        return source
    if any(peer in network for network in trusted_proxies):
        forwarded = request.headers.get('X-Real-IP')
        if forwarded:
            try:
                return str(ipaddress.ip_address(forwarded))
            except ValueError:
                raise web.HTTPBadRequest(text='invalid_proxy_source')
    return str(peer)


def payload_bytes(value):
    # Count retained strings, including signatures/IDs. Object overhead is also
    # bounded by mailbox count and the protocol's per-mailbox batch limits.
    if isinstance(value, str):
        return len(value.encode('utf-8'))
    if isinstance(value, dict):
        return sum(payload_bytes(v) for v in value.values())
    if isinstance(value, list):
        return sum(payload_bytes(v) for v in value)
    return 0


class ByteBudget:
    def __init__(self, limit):
        self.limit, self.used = limit, 0

    def change(self, delta):
        if self.used + delta > self.limit:
            raise unavailable('mailbox_bytes_capacity')
        self.used += delta


class Mailboxes(dict):
    def __init__(self, total, per_scope, scope, budget, clock=time.monotonic):
        super().__init__()
        self.total, self.per_scope, self.scope = total, per_scope, scope
        self.budget, self.clock = budget, clock
        self.counts, self.weights = Counter(), {}

    def __setitem__(self, key, value):
        scope = self.scope(key)
        if key not in self and (len(self) >= self.total or self.counts[scope] >= self.per_scope):
            raise unavailable('mailbox_capacity')
        weight = payload_bytes(value)
        self.budget.change(weight - self.weights.get(key, 0))
        if key not in self:
            self.counts[scope] += 1
        self.weights[key] = weight
        super().__setitem__(key, value)

    def __delitem__(self, key):
        super().__delitem__(key)
        self.budget.change(-self.weights.pop(key))
        scope = self.scope(key)
        self.counts[scope] -= 1
        if not self.counts[scope]:
            del self.counts[scope]

    def get(self, key, default=None):
        value = super().get(key)
        if value is not None and value['deadline'] <= self.clock():
            del self[key]
            return default
        return value if value is not None else default

    def pop(self, key, default=None):
        value = self.get(key)
        if value is not None:
            del self[key]
            return value
        return default

    def update_entry(self, key, **changes):
        self[key] = dict(self[key], **changes)

    def expire(self, on_expired=None):
        now = self.clock()
        for key, value in list(self.items()):
            if value['deadline'] <= now:
                if on_expired:
                    on_expired(key, value)
                del self[key]
