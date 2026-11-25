"""One authenticated service per ledger; parent stdin EOF terminates the owner."""
import argparse
import asyncio
import concurrent.futures
import hmac
import json
import os
from pathlib import Path
import secrets
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, ProxyHandler, build_opener

from .catalog import Catalog
from .engine import Engine


def lock_file(path):
    file = open(path, "a+b")
    try:
        if os.name == "nt":
            import msvcrt
            file.seek(0)
            if not file.read(1):
                file.write(b"0")
                file.flush()
            file.seek(0)
            msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        return file
    except OSError:
        file.close()
        return None


class Service:
    def __init__(self, root, config):
        self.config = config
        self.catalog = Catalog(root / "catalogs" / config["fingerprint"])
        self.engine = Engine(root / "indexes", config)
        self.busy = threading.Lock()
        self.loop = asyncio.new_event_loop()
        threading.Thread(target=self.worker, daemon=True).start()

    def worker(self):
        while True:
            time.sleep(0.5)
            if not self.config["enabled"] or not self.busy.acquire(blocking=False):
                continue
            row = None
            try:
                row = self.catalog.claim()
                if row:
                    future = asyncio.run_coroutine_threadsafe(self.engine.process(row), self.loop)
                    data_id = future.result()
                    self.catalog.finish(row, data_id=data_id)
            except Exception as error:
                if row:
                    # Exception class only: third-party errors can contain prompts or keys.
                    self.catalog.finish(row, error=type(error).__name__)
                    print("Task recall processing failed: " + type(error).__name__, file=sys.stderr, flush=True)
            finally:
                self.busy.release()

    def query(self, body):
        query = body.get("query", "")
        if not isinstance(query, str) or not 0 < len(query) <= 4000:
            raise ValueError("Invalid recall query")
        routes = {"fulltext":self.catalog.lexical(query)}
        errors = {}
        state = "disabled" if not self.config["enabled"] else "indexing"
        if self.config["enabled"] and self.busy.acquire(blocking=False):
            try:
                future = asyncio.run_coroutine_threadsafe(self.engine.search(query, self.catalog.ready()), self.loop)
                extra, errors = future.result(timeout=55)
                routes.update(extra)
                state = "degraded" if errors else "ready"
            except concurrent.futures.TimeoutError:
                future.cancel()
                errors["semanticGraph"] = "timeout"
                state = "degraded"
            except Exception as error:
                errors["semanticGraph"] = type(error).__name__
                state = "degraded"
            finally:
                self.busy.release()
        status = self.catalog.status()
        if self.config["enabled"] and status["counts"].get("failed"):
            state = "degraded"
        elif self.config["enabled"] and status["pending"] and state == "ready":
            state = "indexing"
        scores, hits = {}, {}
        for route, rows in routes.items():
            for rank, row in enumerate(rows, 1):
                rid = row["runId"]
                scores[rid] = scores.get(rid, 0) + {"semantic":1.0,"graph":0.6,"fulltext":0.8}[route] / (10 + rank)
                hit = hits.setdefault(rid, {"runId":rid,"contentHash":row["contentHash"],"evidence":[]})
                hit["evidence"].append({"route":route,"rank":rank})
        results = sorted(hits.values(), key=lambda hit:(-scores[hit["runId"]],hit["runId"]))
        for hit in results:
            hit["score"] = scores[hit["runId"]]
        return {"hits":results[:60], "recall":{"state":state,"routes":list(routes),"errors":errors,**status}}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", required=True)
    args = parser.parse_args()
    root = Path(args.data_dir).resolve() / "tasks/recall"
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    handshake = sys.stdout
    # Nothing from Cognee/dependencies may corrupt the one-line startup protocol.
    sys.stdout = sys.stderr
    owner = lock_file(root / "service.lock")
    if not owner:
        opener = build_opener(ProxyHandler({}))
        for _ in range(100):
            try:
                info = json.loads((root / "runtime.json").read_text())
                request = Request(info["endpoint"] + "/health", headers={"Authorization":"Bearer " + info["token"]})
                with opener.open(request, timeout=0.3) as response:
                    if json.load(response)["protocol"] == 1:
                        handshake.write(json.dumps({**info,"owned":False}) + "\n")
                        handshake.flush()
                        return
            except Exception:
                time.sleep(0.1)
        raise RuntimeError("Existing recall service unavailable")
    # Sensitive runtime credentials arrive over stdin, never argv or files.
    config_line = sys.stdin.readline(65536)
    config = json.loads(config_line)
    service = Service(root, config)
    token = secrets.token_hex(32)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def dispatch(self):
            if self.headers.get_all("Host") != [f"127.0.0.1:{server.server_port}"] or self.headers.get_all("Origin"):
                self.send_error(403)
                return
            headers = self.headers.get_all("Authorization") or []
            if len(headers) != 1 or not hmac.compare_digest(headers[0], "Bearer " + token):
                self.send_error(401)
                return
            try:
                if self.command == "GET" and self.path == "/health":
                    result = {"protocol":1,"fingerprint":config["fingerprint"],**service.catalog.status()}
                elif self.command == "POST":
                    size = int(self.headers.get("Content-Length", "0"))
                    if not 0 < size <= 32 * 1024 * 1024:
                        raise ValueError("Invalid body size")
                    body = json.loads(self.rfile.read(size))
                    if self.path == "/sync":
                        result = service.catalog.sync(body["documents"])
                    elif self.path == "/query":
                        result = service.query(body)
                    elif self.path == "/graph":
                        if not service.busy.acquire(blocking=False):
                            self.send_error(409, "Index busy")
                            return
                        try:
                            result = asyncio.run_coroutine_threadsafe(service.engine.graph(semantic=bool(body.get("semantic"))), service.loop).result(timeout=45)
                        finally:
                            service.busy.release()
                    else:
                        self.send_error(404)
                        return
                else:
                    self.send_error(404)
                    return
                raw = json.dumps(result, ensure_ascii=False).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
            except (ValueError, KeyError, TypeError):
                self.send_error(400, "Invalid recall request")
            except Exception:
                self.send_error(503, "Recall operation unavailable")
        do_POST = dispatch
        do_GET = dispatch

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    info = {"protocol":1,"endpoint":f"http://127.0.0.1:{server.server_port}","token":token}
    temp = root / "runtime.tmp"
    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as file:
        json.dump(info, file)
    os.replace(temp, root / "runtime.json")
    handshake.write(json.dumps({**info,"owned":True}) + "\n")
    handshake.flush()

    def watch_parent():
        while sys.stdin.buffer.read(1):
            pass
        # Native model/database calls may not cancel cooperatively. Process exit
        # releases the OS lock; catalog recovers processing work on next launch.
        os._exit(0)
    threading.Thread(target=watch_parent, daemon=True).start()
    # Native Arrow/Pandas initialization must run on the main interpreter
    # thread; background-thread cold imports can exceed the preview deadline.
    threading.Thread(target=server.serve_forever, daemon=True).start()
    asyncio.set_event_loop(service.loop)
    service.loop.run_forever()
