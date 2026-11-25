"""Loopback-only model adapter with inspectable request/response evidence.

No models are downloaded here. Inference is forwarded to the user's running
MLX Core. Credentials are never logged, and vector arrays are summarized.
"""

from __future__ import annotations

from contextlib import nullcontext
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import hashlib
from pathlib import Path
import threading
import time
from urllib.error import HTTPError
from urllib.parse import urlparse
from urllib.request import Request, urlopen


def loopback_base(value: str) -> str:
    parsed = urlparse(value)
    if parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost", "::1"}:
        raise ValueError("This probe only sends model inputs to an HTTP loopback endpoint")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("Use a loopback API base URL without credentials or query parameters")
    return value.rstrip("/")


class AuditedGateway:
    def __init__(self, upstream: str, evidence: Path, port: int = 0, reuse: Path | None = None):
        self.upstream = loopback_base(upstream)
        self.evidence = evidence
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.counter = max((int(path.stem) for path in self.evidence.iterdir()
                            if path.stem.isdigit()), default=0)
        self.lock = threading.Lock()
        self.chat_lock = threading.Semaphore(1)
        self.reusable = {}
        if reuse:
            for path in sorted(reuse.glob("*.json")):
                saved = json.loads(path.read_text())
                if saved.get("route") == "chat/completions" and saved.get("status") == 200 and saved.get("response"):
                    self.reusable[request_hash(saved["request"])] = (saved["response"], str(path.resolve()))
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):
                self.forward(None)

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
                self.forward(body)

            def forward(self, body):
                route = self.path.removeprefix("/v1/")
                if route not in {"models", "chat/completions", "embeddings"}:
                    self.send_error(404)
                    return
                if route == "chat/completions" and body is not None:
                    body.setdefault("max_tokens", 4096)
                    # Cognee's native fallback already places the complete JSON
                    # schema in the prompt and validates the answer with Pydantic.
                    # This MLX build's json_object grammar is much slower than
                    # ordinary decoding. Keep real json_schema requests intact.
                    response_format = body.get("response_format") or {}
                    if response_format.get("type") == "json_object":
                        body.pop("response_format")
                        adaptation = "Prompt-provided schema; Pydantic validation in Cognee; no decoder grammar"
                    else:
                        adaptation = "Unchanged response format"
                else:
                    adaptation = None
                with owner.lock:
                    owner.counter += 1
                    number = owner.counter
                started = time.perf_counter()
                entry = {"number": number, "route": route, "request": body, "adaptation": adaptation}
                pending = owner.evidence / f"{number:04d}.pending"
                pending.write_text(json.dumps(entry, ensure_ascii=False, indent=2), encoding="utf-8")
                try:
                    def send(payload):
                        request = Request(owner.upstream + "/" + route,
                                          data=json.dumps(payload, ensure_ascii=False).encode() if payload is not None else None,
                                          headers={"Content-Type": "application/json", "Authorization": "Bearer mlx-local"})
                        with urlopen(request, timeout=600) as response:
                            return json.load(response)

                    cached = owner.reusable.get(request_hash(body)) if route == "chat/completions" else None
                    if cached:
                        parsed, entry["reusedFrom"] = cached
                    elif route == "embeddings" and isinstance(body.get("input"), list) and len(body["input"]) > 8:
                        parsed = {"object": "list", "model": body["model"], "data": [], "usage": {}}
                        entry["upstreamBatchSizes"] = []
                        for start in range(0, len(body["input"]), 8):
                            batch = body["input"][start:start + 8]
                            result = send({**body, "input": batch})
                            entry["upstreamBatchSizes"].append(len(batch))
                            parsed["data"].extend({**row, "index": start + row["index"]} for row in result["data"])
                            for key, value in (result.get("usage") or {}).items():
                                if isinstance(value, int):
                                    parsed["usage"][key] = parsed["usage"].get(key, 0) + value
                    else:
                        gate = owner.chat_lock if route == "chat/completions" else nullcontext()
                        with gate:
                            parsed = send(body)
                    raw, status = json.dumps(parsed, ensure_ascii=False).encode(), 200
                    entry["response"] = (parsed if route != "embeddings" else {
                        "model": parsed.get("model"), "usage": parsed.get("usage"),
                        "dimensions": [len(row["embedding"]) for row in parsed.get("data", [])],
                    })
                except HTTPError as error:
                    raw, status = error.read(), error.code
                    entry["error"] = raw.decode(errors="replace")
                except Exception as error:
                    status = 502
                    entry["error"] = f"{type(error).__name__}: {error}"
                    raw = json.dumps({"error": {"message": entry["error"]}}).encode()
                entry.update(status=status, seconds=time.perf_counter() - started)
                (owner.evidence / f"{number:04d}.json").write_text(
                    json.dumps(entry, ensure_ascii=False, indent=2), encoding="utf-8"
                )
                pending.unlink(missing_ok=True)
                try:
                    self.send_response(status)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self) -> str:
        self.thread.start()
        return f"http://127.0.0.1:{self.server.server_port}/v1"

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def request_hash(body: dict) -> str:
    return hashlib.sha256(json.dumps(body, ensure_ascii=False, sort_keys=True).encode()).hexdigest()


def assess_calls(folder: Path) -> dict:
    """Check literal evidence presence, not whether it entails a claimed fact."""
    graphs, unsupported, errors, policy, seconds, usage = [], [], [], [], 0.0, {}
    embedding_dimensions, embedding_calls, max_embedding_tokens = set(), 0, 0
    reused_calls, fresh_calls = 0, 0
    for path in sorted(folder.glob("*.json")):
        call = json.loads(path.read_text())
        if call.get("error"):
            errors.append({"call": path.name, "error": call["error"]})
        if call["route"] == "embeddings":
            embedding_calls += 1
            response = call.get("response", {})
            embedding_dimensions.update(response.get("dimensions", []))
            max_embedding_tokens = max(max_embedding_tokens, (response.get("usage") or {}).get("prompt_tokens", 0))
        if call["route"] != "chat/completions":
            continue
        if call.get("reusedFrom"):
            reused_calls += 1
        else:
            fresh_calls += 1
        seconds += call["seconds"]
        for key, value in (call.get("response", {}).get("usage") or {}).items():
            if isinstance(value, int):
                usage[key] = usage.get(key, 0) + value
        for choice in call.get("response", {}).get("choices", []):
            content = choice.get("message", {}).get("content", "")
            try:
                candidate = content.strip()
                fenced = candidate.startswith("```") and candidate.endswith("```")
                if fenced:
                    candidate = candidate.split("\n", 1)[1].rsplit("```", 1)[0].strip()
                graph = json.loads(candidate)
            except (ValueError, TypeError):
                continue
            if not isinstance(graph, dict) or "nodes" not in graph or "edges" not in graph:
                continue
            allowed_relations = {"affects", "addresses", "depends_on", "uses", "implements", "changes", "verifies", "supersedes"}
            invalid_relations = sorted({edge.get("relationship_name", "") for edge in graph["edges"]} - allowed_relations)
            if len(graph["nodes"]) > 8 or len(graph["edges"]) > 8 or invalid_relations:
                policy.append({"call": path.name, "nodes": len(graph["nodes"]), "edges": len(graph["edges"]),
                               "unexpectedRelations": invalid_relations})
            source = "\n".join(str(message.get("content", "")) for message in call["request"]["messages"]
                               if message.get("role") == "user")
            import re
            for edge in graph["edges"]:
                quotes = re.findall(r"「([^」]+)」", edge.get("description") or "")
                if not quotes or any(quote not in source for quote in quotes):
                    unsupported.append({"call": path.name, "edge": edge, "quotes": quotes})
            graphs.append({"call": path.name, "source": source, "fencedJson": fenced,
                           "reusedFrom": call.get("reusedFrom"), **graph})
    return {"extractions": graphs, "missingOrNonliteralEvidence": unsupported,
            "promptPolicyViolations": policy,
            "embeddingCalls": embedding_calls, "embeddingDimensions": sorted(embedding_dimensions),
            "maxEmbeddingBatchTokens": max_embedding_tokens,
            "reusedLLMCalls": reused_calls, "freshLLMCalls": fresh_calls,
            "errors": errors, "llmSeconds": seconds, "usage": usage,
            "note": "Literal evidence checks do not establish factual entailment or human verification."}
