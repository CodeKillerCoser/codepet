"""Exercise the process boundary without a model or Cognee import."""
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from urllib.error import HTTPError
from urllib.request import Request, build_opener, ProxyHandler
import uuid


class ServiceTests(unittest.TestCase):
    def test_waiting_attacher_takes_over_when_owner_exits_before_ready(self):
        from codepet_recall.lifecycle import lock_file

        with tempfile.TemporaryDirectory() as temporary:
            data = Path(temporary)
            root = data / "tasks/recall"
            root.mkdir(parents=True)
            owner = lock_file(root / "service.lock")
            self.assertIsNotNone(owner)
            child = subprocess.Popen(
                [sys.executable, "-I", "-B", str(Path(__file__).resolve().parents[1] / "main.py"),
                 "--data-dir", str(data)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                child.stdin.write(json.dumps({"enabled":False,"fingerprint":"handover"}).encode() + b"\n")
                child.stdin.flush()
                # Ensure the child has reached the non-owner path, then simulate
                # an exiting owner that never published a healthy endpoint.
                time.sleep(0.5)
                owner.close()
                output = queue.Queue()
                threading.Thread(target=lambda: output.put(child.stdout.readline()), daemon=True).start()
                line = output.get(timeout=4)
                self.assertTrue(json.loads(line)["owned"])
            finally:
                owner.close()
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
                for stream in (child.stdin, child.stdout, child.stderr):
                    stream.close()

    def test_auth_singleton_relocation_clean_environment_and_parent_eof(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = Path(__file__).resolve().parents[1]
            bundle = os.environ.get("CODEPET_RECALL_TEST_BUNDLE")
            if bundle:
                app = Path(bundle)
                manifest = json.loads((app / "manifest.json").read_text())
                python = str(app / manifest["pythonPath"])
            else:
                app = root / "独立 sidecar"
                app.mkdir()
                shutil.copy2(source / "main.py", app / "main.py")
                shutil.copytree(source / "codepet_recall", app / "codepet_recall")
                python = sys.executable
            config = {"enabled":False,"fingerprint":"test","llm":{},"embedding":{}}
            processes = []

            def start():
                env = {**os.environ,"PYTHONPATH":"/invalid","PYTHONHOME":"/invalid","PATH":os.defpath,
                       "HTTP_PROXY":"http://127.0.0.1:1","HTTPS_PROXY":"http://127.0.0.1:1","NO_PROXY":""}
                child = subprocess.Popen([python,"-I","-B",str(app / "main.py"),"--data-dir",str(root / "data")],
                    stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,cwd=root,env=env)
                processes.append(child)
                child.stdin.write(json.dumps(config).encode() + b"\n"); child.stdin.flush()
                output = queue.Queue()
                threading.Thread(target=lambda:output.put(child.stdout.readline()),daemon=True).start()
                line = output.get(timeout=15)
                self.assertTrue(line, child.stderr.read().decode() if child.poll() is not None else "missing handshake")
                return child,json.loads(line)

            opener = build_opener(ProxyHandler({}))
            try:
                owner,info = start()
                self.assertTrue(info["owned"])

                def request(path, body=None, headers=None):
                    req = Request(info["endpoint"] + path, data=json.dumps(body).encode() if body is not None else None,
                        headers={"Authorization":"Bearer " + info["token"],"Content-Type":"application/json",**(headers or {})})
                    with opener.open(req,timeout=5) as response:
                        return json.load(response)

                for headers,code in [({"Authorization":"Bearer wrong"},401),({"Origin":"https://evil.invalid"},403),({"Host":"evil.invalid"},403)]:
                    with self.assertRaises(HTTPError) as caught:
                        request("/health",headers=headers)
                    self.assertEqual(caught.exception.code,code)
                text = "统一侧栏抽屉预览"
                doc = {"runId":str(uuid.uuid4()),"taskId":str(uuid.uuid4()),"eligible":True,
                       "text":text,"contentHash":hashlib.sha256(text.encode()).hexdigest()}
                request("/sync",{"documents":[doc]})
                jobs = request("/history", {"action":"list"})
                self.assertEqual(jobs["counts"]["queued"], 1)
                job = jobs["items"][0]["id"]
                request("/history", {"action":"cancel", "jobId":job})
                request("/sync", {"documents":[doc]})
                detail = request("/history", {"action":"detail", "jobId":job})
                self.assertEqual(detail["state"], "cancelled")
                self.assertFalse(detail["usageTracked"])
                with self.assertRaises(HTTPError) as caught:
                    request("/history", {"action":"cancel", "jobId":job})
                self.assertEqual(caught.exception.code, 409)
                operation = str(uuid.uuid4())
                queued = request("/history", {"action":"enqueue", "taskId":doc["taskId"], "operationId":operation})
                self.assertEqual(queued["queued"], 1)
                self.assertNotEqual(queued["jobIds"], [job])
                self.assertEqual(request("/history", {"action":"enqueue", "taskId":doc["taskId"], "operationId":operation}), queued)
                attached,other = start()
                self.assertFalse(other["owned"])
                self.assertEqual(info["endpoint"],other["endpoint"])
                attached.wait(timeout=5)
                result = request("/query",{"query":"侧栏抽屉"})
                self.assertEqual(result["hits"][0]["runId"],doc["runId"])
                self.assertEqual(result["recall"]["state"],"disabled")
                owner.stdin.close()
                owner.wait(timeout=5)
                restarted,next_info = start()
                self.assertTrue(next_info["owned"])
                info = next_info
                self.assertEqual(request("/query",{"query":"侧栏"})["hits"][0]["runId"],doc["runId"])
                restarted.stdin.close(); restarted.wait(timeout=5)
            finally:
                for child in processes:
                    if child.poll() is None:
                        child.kill(); child.wait(timeout=5)
                    for stream in (child.stdin,child.stdout,child.stderr):
                        if stream and not stream.closed:
                            stream.close()


if __name__ == "__main__":
    unittest.main()
