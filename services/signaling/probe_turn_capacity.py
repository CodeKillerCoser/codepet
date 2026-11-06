"""On-server authenticated UDP allocation probe; all reservations are released."""
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import socket
import struct
import time

def attribute(kind, value):
    return struct.pack('!HH', kind, len(value)) + value + b'\0' * (-len(value) % 4)

def request(sock, target, kind, attributes, key=None):
    tx = os.urandom(12)
    body = b''.join(attributes)
    header = struct.pack('!HHI', kind, len(body) + (24 if key else 0), 0x2112A442) + tx
    if key:
        body += attribute(8, hmac.new(key, header + body, hashlib.sha1).digest())
    sock.sendto(header + body, target)
    response = sock.recv(4096)
    if response[8:20] != tx:
        raise RuntimeError('Unexpected STUN transaction')
    attrs = {}
    offset = 20
    while offset + 4 <= len(response):
        tag, length = struct.unpack('!HH', response[offset:offset+4])
        attrs[tag] = response[offset+4:offset+4+length]
        offset += 4 + length + (-length % 4)
    return struct.unpack('!H', response[:2])[0], attrs

def main():
    config = json.loads(Path('/etc/codepet-signal/config.json').read_text())
    target = (config['publicIp'], 3478)
    reservations = []
    transport = attribute(0x19, b'\x11\0\0\0')
    try:
        for index in range(12):
            username = f'{int(time.time())+3600}:capacity-probe-{os.getpid()}-{index//8}'.encode()
            password = base64.b64encode(hmac.new(config['turnSecret'].encode(), username, hashlib.sha1).digest())
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.settimeout(3)
            sock.bind((config['publicIp'], 0))
            try:
                _, challenge = request(sock, target, 3, [transport])
                realm, nonce = challenge[0x14], challenge[0x15]
                auth = [attribute(6, username), attribute(0x14, realm), attribute(0x15, nonce)]
                key = hashlib.md5(username+b':'+realm+b':'+password).digest()
                code, attrs = request(sock, target, 3, [transport]+auth, key)
                if code != 0x103 or 0x16 not in attrs:
                    error = attrs.get(9, b'\0\0\0\0')
                    raise RuntimeError(f'Allocation {index+1} rejected: {(error[2]&7)*100+error[3]}')
                reservations.append((sock, auth, key))
            except Exception:
                sock.close()
                raise
        print('PASS: 12 simultaneous authenticated TURN allocations across two users')
    finally:
        for sock, auth, key in reservations:
            try:
                request(sock, target, 4, [attribute(0x0d, struct.pack('!I', 0))]+auth, key)
            finally:
                sock.close()
        print(f'Released {len(reservations)} temporary allocations')

if __name__ == '__main__': main()
