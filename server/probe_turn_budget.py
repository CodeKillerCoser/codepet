"""Verify the actual packet budget in an isolated Linux network namespace."""
import json
import os
from pathlib import Path
import pwd
import socket
import subprocess
import sys

from deploy_management import budget_rules


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def counters():
    data = json.loads(run('nft', '-j', 'list', 'table', 'inet', 'codepet_turn_budget',
                          capture_output=True, text=True).stdout)
    return [item['counter']['packets'] for entry in data['nftables'] if 'rule' in entry
            for item in entry['rule']['expr'] if 'counter' in item]


def send():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        for _ in range(100):
            try:
                sock.sendto(b'x' * 1024, ('192.0.2.2', 9999))
            except PermissionError:
                # A local OUTPUT drop can surface as EPERM/EACCES to sendto.
                # Kernel counters below must still prove the intended rule hit.
                pass


def main():
    if sys.argv[1:] != ['--inside']:
        run('unshare', '--net', sys.executable, str(Path(__file__).resolve()), '--inside')
        return
    # No host routes/interfaces/firewall are touched; destination is unreachable
    # outside this namespace. A tiny test rate exercises the real kernel policer.
    run('ip', 'link', 'add', 'cpbudget', 'type', 'dummy')
    run('ip', 'addr', 'add', '192.0.2.1/24', 'dev', 'cpbudget')
    run('ip', 'link', 'set', 'cpbudget', 'up')
    run('nft', '-f', '-', input=budget_rules(1000), text=True)
    send()  # Other services/users must not consume or hit the TURN budget.
    assert counters() == [0, 0], counters()
    pid = os.fork()
    if pid == 0:
        try:
            os.setuid(pwd.getpwnam('coturn').pw_uid)
            send()
            os._exit(0)
        except BaseException:
            os._exit(1)
    _, status = os.waitpid(pid, 0)
    assert status == 0, status
    seen, dropped = counters()
    assert seen == 100 and dropped > 0, (seen, dropped)
    print(f'PASS: isolated nftables budget, coturn packets={seen}, dropped={dropped}; other UID unaffected')


if __name__ == '__main__':
    main()
