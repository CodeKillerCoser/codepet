#!/usr/bin/env python3
"""Measure old/new conversation payloads and RFC 7692 wire compression.

The benchmark talks only to built Provider binaries over JSON-RPC stdio. It keeps
conversation text in memory, writes aggregate metrics only, and models the
negotiated Gateway setting: permessage-deflate at zlib's default level with
server_no_context_takeover. The WebSocket payload estimate excludes TLS/TCP/IP.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import queue
import subprocess
import threading
import time
import zlib
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Any


ROUTE = {
    "deviceId": "payload-benchmark-device",
    "providerPluginId": "dev.codepet.codex",
    "providerInstanceId": "payload-benchmark-codex",
}

CASES = (
    ("A-40", "01a06325-a5b4-7eb1-ab79-fd82e7e649a6", 40),
    ("A-20", "01a06325-a5b4-7eb1-ab79-fd82e7e649a6", 20),
    ("A-10", "01a06325-a5b4-7eb1-ab79-fd82e7e649a6", 10),
    ("A-1", "01a06325-a5b4-7eb1-ab79-fd82e7e649a6", 1),
    ("B-40", "01a04f08-c0f4-7e32-a00a-834191cda4af", 40),
    ("C-40", "01a068e3-a391-78a2-975f-f3859c58f84a", 40),
)


class ProviderProcess:
    def __init__(self, executable: Path) -> None:
        self._process = subprocess.Popen(
            [str(executable)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        assert self._process.stdin is not None
        assert self._process.stdout is not None
        self._messages: queue.Queue[tuple[dict[str, Any], str]] = queue.Queue()
        self._buffered: list[tuple[dict[str, Any], str]] = []
        threading.Thread(target=self._read_stdout, daemon=True).start()

    def _read_stdout(self) -> None:
        assert self._process.stdout is not None
        for line in self._process.stdout:
            self._messages.put((json.loads(line), line.rstrip("\n")))

    def request(
        self,
        request_id: str,
        method: str,
        params: dict[str, Any],
        timeout_seconds: float = 180,
    ) -> tuple[dict[str, Any], str]:
        assert self._process.stdin is not None
        request = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }
        self._process.stdin.write(compact_json(request).decode("utf-8") + "\n")
        self._process.stdin.flush()
        message, raw = self._receive(request_id, timeout_seconds)
        if "error" in message:
            raise RuntimeError(f"{method} failed: {message['error']}")
        return message["result"], raw

    def _receive(
        self, request_id: str, timeout_seconds: float
    ) -> tuple[dict[str, Any], str]:
        for index, (message, raw) in enumerate(self._buffered):
            if message.get("id") == request_id:
                self._buffered.pop(index)
                return message, raw
        deadline = time.monotonic() + timeout_seconds
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"timed out waiting for {request_id}")
            try:
                message, raw = self._messages.get(timeout=remaining)
            except queue.Empty as error:
                stderr = ""
                if self._process.poll() is not None and self._process.stderr is not None:
                    stderr = self._process.stderr.read()
                raise TimeoutError(
                    f"provider exited={self._process.poll()} stderr={stderr}"
                ) from error
            if message.get("id") == request_id:
                return message, raw
            self._buffered.append((message, raw))

    def close(self) -> None:
        try:
            self.request("stop", "instance.stop", {"route": ROUTE}, 30)
            self.request("shutdown", "provider.shutdown", {}, 30)
        finally:
            if self._process.stdin is not None and not self._process.stdin.closed:
                self._process.stdin.close()
            try:
                self._process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=5)


def compact_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def permessage_deflate(payload: bytes) -> bytes:
    compressor = zlib.compressobj(level=zlib.Z_DEFAULT_COMPRESSION, wbits=-15)
    compressed = compressor.compress(payload) + compressor.flush(zlib.Z_SYNC_FLUSH)
    trailer = b"\x00\x00\xff\xff"
    if not compressed.endswith(trailer):
        raise AssertionError("RFC 7692 sync-flush trailer missing")
    return compressed[: -len(trailer)]


def server_frame_overhead(payload_bytes: int) -> int:
    if payload_bytes <= 125:
        return 2
    if payload_bytes <= 65_535:
        return 4
    return 10


def count_shape(value: Any) -> tuple[Counter[str], int, int, int]:
    kinds: Counter[str] = Counter()
    truncations = 0
    original_bytes = 0
    retained_bytes = 0

    def visit(node: Any) -> None:
        nonlocal truncations, original_bytes, retained_bytes
        if isinstance(node, dict):
            kind = node.get("kind")
            if isinstance(kind, str):
                kinds[kind] += 1
            truncation = node.get("truncation")
            if isinstance(truncation, dict):
                truncations += 1
                original = truncation.get("originalBytes")
                if isinstance(original, int):
                    original_bytes += original
                retained = truncation.get("retainedBytes")
                if isinstance(retained, int):
                    retained_bytes += retained
            for child in node.values():
                visit(child)
        elif isinstance(node, list):
            for child in node:
                visit(child)

    visit(value)
    return kinds, truncations, original_bytes, retained_bytes


def measure_case(result: dict[str, Any], raw_frame: str) -> dict[str, Any]:
    result_payload = compact_json(result)
    websocket_payload = raw_frame.encode("utf-8")
    compressed = permessage_deflate(websocket_payload)
    kinds, truncations, original_bytes, retained_bytes = count_shape(result)
    items = result.get("items")
    return {
        "resultBytes": len(result_payload),
        "jsonRpcBytes": len(websocket_payload),
        "deflatePayloadBytes": len(compressed),
        "webSocketWireBytes": len(compressed) + server_frame_overhead(len(compressed)),
        "compressionRatio": round(len(compressed) / len(websocket_payload), 6),
        "wireReductionPercent": round(
            (1 - (len(compressed) + server_frame_overhead(len(compressed))) / len(websocket_payload))
            * 100,
            3,
        ),
        "itemCount": len(items) if isinstance(items, list) else None,
        "conversationCount": len(result.get("conversations", []))
        if isinstance(result.get("conversations"), list)
        else None,
        "nextCursorPresent": bool((result.get("pageInfo") or {}).get("nextCursor")),
        "truncatedValueCount": truncations,
        "originalTruncatedBytes": original_bytes,
        "retainedTruncatedBytes": retained_bytes,
        "omittedTruncatedBytes": original_bytes - retained_bytes,
        "selectedKindCounts": {
            key: kinds[key]
            for key in (
                "message",
                "reasoning",
                "command",
                "file-change",
                "tool",
                "approval",
                "unknown",
                "output",
                "structured-json",
            )
            if kinds[key]
        },
    }


def benchmark(label: str, provider_path: Path, codex_path: Path) -> dict[str, Any]:
    provider = ProviderProcess(provider_path)
    try:
        provider.request(
            f"{label}-initialize",
            "provider.initialize",
            {
                "hostClientId": "payload-benchmark",
                "hostDeviceId": ROUTE["deviceId"],
                "hostVersion": "payload-benchmark",
                "supportedVersions": {"minVersion": 1, "maxVersion": 1},
            },
        )
        provider.request(
            f"{label}-create",
            "instance.create",
            {
                "route": ROUTE,
                "instanceKind": "codex",
                "displayName": "Payload benchmark",
                "settings": {
                    "appServerExecutable": str(codex_path),
                    "appServerArgs": ["app-server", "--listen", "stdio://"],
                },
            },
        )
        provider.request(f"{label}-start", "instance.start", {"route": ROUTE}, 180)
        measurements: dict[str, Any] = {}
        listed, listed_raw = provider.request(
            f"{label}-list-100",
            "conversation.list",
            {"route": ROUTE, "limit": 100, "projectFilter": {"kind": "all"}},
        )
        measurements["list-100"] = measure_case(listed, listed_raw)
        for case_label, conversation_id, limit in CASES:
            result, raw = provider.request(
                f"{label}-{case_label}",
                "conversation.get",
                {
                    "conversation": {**ROUTE, "nativeResourceId": conversation_id},
                    "limit": limit,
                },
            )
            measurements[case_label] = measure_case(result, raw)
        return measurements
    finally:
        provider.close()


def comparisons(old: dict[str, Any], new: dict[str, Any]) -> dict[str, Any]:
    rows: dict[str, Any] = {}
    for case_label in old.keys() & new.keys():
        old_case = old[case_label]
        new_case = new[case_label]
        rows[case_label] = {
            "jsonRpcReductionPercent": round(
                (1 - new_case["jsonRpcBytes"] / old_case["jsonRpcBytes"]) * 100, 3
            ),
            "compressedWireReductionPercent": round(
                (1 - new_case["webSocketWireBytes"] / old_case["webSocketWireBytes"])
                * 100,
                3,
            ),
            "oldCompressedWireBytes": old_case["webSocketWireBytes"],
            "newCompressedWireBytes": new_case["webSocketWireBytes"],
        }
    return dict(sorted(rows.items()))


def file_identity(path: Path) -> dict[str, Any]:
    resolved = path.resolve()
    digest = hashlib.sha256()
    with resolved.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "path": str(resolved),
        "bytes": resolved.stat().st_size,
        "sha256": digest.hexdigest(),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--old-provider", required=True, type=Path)
    parser.add_argument("--new-provider", required=True, type=Path)
    parser.add_argument("--codex", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    old = benchmark("old", arguments.old_provider.resolve(), arguments.codex.resolve())
    new = benchmark("new", arguments.new_provider.resolve(), arguments.codex.resolve())
    output = {
        "provenance": {
            "generatedAtLocal": datetime.now().astimezone().isoformat(timespec="seconds"),
            "codexVersion": subprocess.run(
                [str(arguments.codex.resolve()), "--version"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip(),
            "oldProvider": file_identity(arguments.old_provider),
            "newProvider": file_identity(arguments.new_provider),
        },
        "method": {
            "transport": "Provider JSON-RPC stdio response used as Gateway payload proxy",
            "compression": "RFC 7692 permessage-deflate, zlib default level, raw DEFLATE, Z_SYNC_FLUSH trailer removed",
            "contextTakeover": False,
            "webSocketWireExcludes": ["TLS", "TCP", "IP"],
        },
        "old": old,
        "new": new,
        "comparison": comparisons(old, new),
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_bytes(json.dumps(output, ensure_ascii=False, indent=2).encode("utf-8") + b"\n")
    print(json.dumps(output, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
