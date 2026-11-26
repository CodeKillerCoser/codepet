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

from .catalog import Catalog
from .engine import Engine
from .history import QueueConflict, read_history
from .model_trace import attempt_id
from .lifecycle import acquire_service, StartupUnavailable
from .published_preview import PublishedPreview


class PreviewBusy(Exception):
    pass


class Service:
    def __init__(self, root, config):
        self.config = config
        self.root = root
        models = {key:config.get(key, {}).get("model", "") for key in ("llm", "embedding")}
        self.catalog = Catalog(root / "catalogs" / config["fingerprint"], models, bool(config.get("usageTracking")))
        self.engine = Engine(root / "indexes", config)
        self.preview = PublishedPreview(root / "indexes" / config["fingerprint"])
        self.preview_dirty = False
        self.preview_pending = False
        self.want_semantic = False
        self.busy = threading.Lock()
        self.loop = asyncio.new_event_loop()

    def worker(self):
        warmup = True
        while True:
            time.sleep(0.5)
            if (not warmup and not self.config["enabled"]) or not self.busy.acquire(blocking=False):
                continue
            row = None
            try:
                if warmup:
                    warmup = False
                    try:
                        asyncio.run_coroutine_threadsafe(self.prepare_preview(), self.loop).result()
                    except Exception as error:
                        print("Task recall preview preparation failed: " + type(error).__name__, file=sys.stderr, flush=True)
                row = self.catalog.claim() if self.config["enabled"] else None
                if row:
                    future = asyncio.run_coroutine_threadsafe(self.process(row), self.loop)
                    data_id, result, graph, projected = future.result()
                    if self.catalog.finish(row, data_id=data_id, result=result):
                        # Publish after the attempt is recorded, before the next
                        # writer invalidates its live graph cache.
                        self.preview.publish(self.engine.dataset, graph, projected)
                        self.preview_dirty = False
                        self.preview_pending = graph is None
            except Exception as error:
                if row:
                    # Exception class only: third-party errors can contain prompts or keys.
                    self.catalog.finish(row, error=type(error).__name__)
                    print("Task recall processing failed: " + type(error).__name__, file=sys.stderr, flush=True)
            finally:
                self.busy.release()

    async def prepare_preview(self):
        # Backfill old indexes without re-running extraction or inference. An
        # existing published snapshot needs no native imports at startup.
        if self.preview.get() is None and (self.preview.root / "dataset.json").is_file():
            graph = await self.engine.graph()
            if graph.get("previewVersion"):
                self.preview.publish(self.engine.dataset, graph)

    async def process(self, row):
        # Capture identity in the pipeline coroutine, so concurrent SDK child tasks
        # carry their own trace rather than a mutable process-wide current job.
        await self.engine.initialize()
        # Existing indexes should be visible even while the first new attempt
        # is running. Native reads stay on the event loop under the writer lock.
        if self.engine.dataset and not self.preview_dirty and self.preview.get() is None:
            try:
                self.preview.publish(self.engine.dataset, await self.engine.graph())
            except Exception:
                pass
        from .model_trace import install
        install(self.config["gatewayEndpoint"], self.config["gatewayToken"])
        token = attempt_id.set(row.get("job_id"))
        try:
            self.preview_dirty = True
            self.preview_pending = True
            data_id = await self.engine.process(row)
            result = graph = projected = None
            if not row["deleted"]:
                self.catalog.phase(row.get("job_id"), "读取加工结果")
                try:
                    result = await self.engine.result(json.loads(row["document"]))
                except Exception:
                    # The index is already committed. A preview failure must not
                    # repeat extraction and charge the model a second time.
                    result = {"description":"索引已更新，结果快照暂不可用", "indexUpdated":True,
                              "summaries":[], "entities":[], "snapshotUnavailable":True}
            try:
                graph = await self.engine.graph()
                if self.want_semantic:
                    projected = await self.engine.graph(semantic=True)
            except Exception:
                # Export/projection failure must not repeat successful model work.
                pass
            return data_id, result, graph, projected
        finally:
            attempt_id.reset(token)

    def graph(self, body):
        semantic = bool(body.get("semantic"))
        if semantic:
            self.want_semantic = True
        snapshot = self.preview.get(semantic)
        if snapshot is not None and (not self.preview_pending or self.busy.locked()) and (not semantic or "semantic_positions" in snapshot):
            return self.graph_response(snapshot, body)
        if not self.busy.acquire(blocking=False):
            if snapshot is not None:
                return self.graph_response(snapshot, body)
            raise PreviewBusy()
        try:
            if self.preview_dirty:
                if snapshot is not None:
                    return self.graph_response(snapshot, body)
                raise PreviewBusy()
            future = asyncio.run_coroutine_threadsafe(self.engine.graph(semantic=semantic), self.loop)
            try:
                graph = future.result(timeout=45)
            except concurrent.futures.TimeoutError:
                future.cancel()
                raise PreviewBusy()
            # Empty indexes have no native version yet; don't persist them.
            if graph.get("previewVersion"):
                self.preview.publish(self.engine.dataset, graph, graph if semantic else None)
                self.preview_pending = False
            return self.graph_response(graph, body)
        finally:
            self.busy.release()

    def graph_response(self, graph, body):
        status = self.catalog.status()
        result = {"previewVersion":graph.get("previewVersion"),
                  "previewStatus":{"updating":bool(status["pending"]), "pending":status["pending"],
                                   "processing":status["counts"].get("processing", 0),
                                   "ready":status["ready"]}}
        semantic_pending = bool(body.get("semantic") and "semantic_positions" not in graph)
        if graph.get("previewVersion") and body.get("knownVersion") == graph["previewVersion"] and (not body.get("semantic") or semantic_pending):
            return {**result, "unchanged":True, "semanticPending":semantic_pending}
        return {**graph, **result, "semanticPending":semantic_pending}

    def history(self, body):
        action = body.get("action", "list")
        if action == "enqueue":
            return self.catalog.enqueue_task(body["taskId"], body["operationId"])
        if action == "cancel":
            return self.catalog.cancel(body["jobId"])
        if action == "retry":
            return self.catalog.retry(body["jobId"])
        if action not in ("list", "detail"):
            raise ValueError("Unknown history action")
        return read_history(self.root / "catalogs", self.config["fingerprint"],
            job=body["jobId"] if action == "detail" else None,
            offset=body.get("offset", 0), limit=body.get("limit", 40), state=body.get("state"))

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
    # Sensitive runtime credentials arrive over stdin, never argv or files.
    config = json.loads(sys.stdin.readline(65536))
    try:
        owner, info = acquire_service(root, config)
    except StartupUnavailable as error:
        handshake.write(json.dumps({"error":str(error)}) + "\n")
        handshake.flush()
        return
    if not owner:
        handshake.write(json.dumps({**info,"owned":False}) + "\n")
        handshake.flush()
        return
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
                    result = {"protocol":1,"fingerprint":config["fingerprint"],
                              "bundleId":config.get("bundleId", ""),**service.catalog.status()}
                elif self.command == "POST":
                    size = int(self.headers.get("Content-Length", "0"))
                    if not 0 < size <= 32 * 1024 * 1024:
                        raise ValueError("Invalid body size")
                    body = json.loads(self.rfile.read(size))
                    if self.path == "/sync":
                        result = service.catalog.sync(body["documents"])
                    elif self.path == "/query":
                        result = service.query(body)
                    elif self.path == "/history":
                        result = service.history(body)
                    elif self.path == "/graph":
                        result = service.graph(body)
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
            except PreviewBusy:
                self.send_error(409, "Preview preparing")
            except QueueConflict:
                self.send_error(409, "Processing queue changed")
            except (ValueError, KeyError, TypeError):
                self.send_error(400, "Invalid recall request")
            except Exception:
                self.send_error(503, "Recall operation unavailable")
        do_POST = dispatch
        do_GET = dispatch

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    info = {"protocol":1,"endpoint":f"http://127.0.0.1:{server.server_port}","token":token}

    def watch_parent():
        while sys.stdin.buffer.read(1):
            pass
        # Native model/database calls may not cancel cooperatively. Process exit
        # releases the OS lock; catalog recovers processing work on next launch.
        os._exit(0)
    async def serve():
        # Advertise only after entering the event loop and starting HTTP. A
        # handshake means requests can be served, not merely that a socket bound.
        threading.Thread(target=server.serve_forever, daemon=True).start()
        temp = root / "runtime.tmp"
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as file:
            json.dump(info, file)
        os.replace(temp, root / "runtime.json")
        handshake.write(json.dumps({**info,"owned":True}) + "\n")
        handshake.flush()
        threading.Thread(target=watch_parent, daemon=True).start()
        # Heavy native imports/export may block the event loop. They must only
        # begin after the ready handshake and HTTP server have been published.
        threading.Thread(target=service.worker, daemon=True).start()
        await asyncio.Event().wait()

    # Native Arrow/Pandas initialization must stay on the main interpreter thread.
    asyncio.set_event_loop(service.loop)
    service.loop.run_until_complete(serve())
