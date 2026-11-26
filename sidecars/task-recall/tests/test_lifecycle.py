"""Startup races use isolated processes and disabled models, never user indexes."""
import json
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
from urllib.error import URLError
from urllib.request import Request, ProxyHandler, build_opener

from codepet_recall import lifecycle


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.children = []
        self.addCleanup(self.stop_children)

    def stop_children(self):
        for child in self.children:
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
            for stream in (child.stdin, child.stdout, child.stderr):
                if not stream.closed:
                    stream.close()

    def spawn(self, setup=""):
        source = Path(__file__).resolve().parents[1]
        script = f"import sys\nsys.path.insert(0, {str(source)!r})\nfrom codepet_recall import server\n{setup}\nserver.main()"
        child = subprocess.Popen([sys.executable, "-I", "-B", "-c", script, "--data-dir", str(self.root)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.children.append(child)
        child.stdin.write(json.dumps({"enabled":False,"fingerprint":"lifecycle","bundleId":"fixture"}).encode() + b"\n")
        child.stdin.flush()
        output = queue.Queue()
        threading.Thread(target=lambda: output.put(child.stdout.readline()), daemon=True).start()
        return child, output

    def hello(self, output):
        return json.loads(output.get(timeout=15))

    def request(self, info, path, body=None):
        request = Request(info["endpoint"] + path, data=json.dumps(body).encode() if body is not None else None,
                          headers={"Authorization":"Bearer " + info["token"],"Content-Type":"application/json"})
        with build_opener(ProxyHandler({})).open(request, timeout=2) as response:
            return json.load(response)

    def test_simultaneous_launches_have_one_ready_owner(self):
        children = [self.spawn() for _ in range(8)]
        infos = [self.hello(output) for _, output in children]
        self.assertEqual(sum(info["owned"] for info in infos), 1)
        self.assertEqual(len({info["endpoint"] for info in infos}), 1)
        for (child, _), info in zip(children, infos):
            self.assertEqual(self.request(info, "/health")["bundleId"], "fixture")
            if not info["owned"]:
                self.assertEqual(child.wait(timeout=2), 0)

    def test_handshake_is_not_published_before_event_loop_setup(self):
        blocked = self.root / "loop-setup"
        release = self.root / "release"
        _, output = self.spawn(f"""
original = server.asyncio.set_event_loop
def delayed(loop):
    server.Path({str(blocked)!r}).touch()
    while not server.Path({str(release)!r}).exists():
        server.time.sleep(0.01)
    original(loop)
server.asyncio.set_event_loop = delayed
async def graph(self, **_):
    return {{"loopReady":server.asyncio.get_running_loop().is_running()}}
server.Engine.graph = graph
""")
        deadline = time.monotonic() + 10
        while not blocked.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(blocked.exists())
        self.assertTrue(output.empty(), "handshake escaped before the loop was ready")
        self.assertFalse((self.root / "tasks/recall/runtime.json").exists())
        release.touch()
        info = self.hello(output)
        self.assertTrue(self.request(info, "/graph", {})["loopReady"])

    def test_retry_reacquires_singleton_instead_of_polling_dead_owner(self):
        owner = object()
        with patch.object(lifecycle, "lock_file", side_effect=[None, owner]) as lock, \
                patch.object(lifecycle, "build_opener") as opener, \
                patch.object(lifecycle.time, "sleep"):
            root = self.root
            (root / "runtime.json").write_text(json.dumps({"endpoint":"http://127.0.0.1:1","token":"fixture"}))
            opener.return_value.open.side_effect = URLError("owner exited")
            actual, info = lifecycle.acquire_service(root, {"fingerprint":"lifecycle"})
        self.assertIs(actual, owner)
        self.assertIsNone(info)
        self.assertEqual(lock.call_count, 2)

    def test_incompatible_live_owner_is_not_attached_or_killed(self):
        (self.root / "runtime.json").write_text(json.dumps({"endpoint":"http://127.0.0.1:1","token":"fixture"}))
        for health in ({"protocol":1}, {"protocol":1,"fingerprint":"lifecycle","bundleId":"old"}):
            with self.subTest(health=health), patch.object(lifecycle, "lock_file", return_value=None), \
                    patch.object(lifecycle, "build_opener") as opener:
                opener.return_value.open.return_value.__enter__.return_value.read.return_value = json.dumps(health).encode()
                with self.assertRaisesRegex(lifecycle.StartupUnavailable, "incompatible_service"):
                    lifecycle.acquire_service(self.root, {"fingerprint":"lifecycle","bundleId":"fixture"}, timeout=0.1)


if __name__ == "__main__":
    unittest.main()
