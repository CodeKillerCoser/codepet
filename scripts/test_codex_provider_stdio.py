#!/usr/bin/env python3
"""Black-box smoke test for a built Codex Provider binary.

The script knows only the public Provider JSON-RPC 2.0 protocol. It does not import Rust
crates or call CodexProvider directly, so it exercises process startup, JSON-lines framing,
typed dispatch, events, the Codex App Server adapter, and orderly shutdown together.
"""

from __future__ import annotations

import argparse
import json
import queue
import subprocess
import tempfile
import threading
from pathlib import Path
from typing import Any


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
        self._messages: queue.Queue[dict[str, Any]] = queue.Queue()
        self._buffered: list[dict[str, Any]] = []
        self._reader = threading.Thread(target=self._read_stdout, daemon=True)
        self._reader.start()

    def _read_stdout(self) -> None:
        assert self._process.stdout is not None
        for line in self._process.stdout:
            self._messages.put(json.loads(line))

    def request(self, request_id: str, method: str, params: dict[str, Any]) -> dict[str, Any]:
        assert self._process.stdin is not None
        self._process.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": method,
                    "params": params,
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        self._process.stdin.flush()
        response = self._receive(lambda message: message.get("id") == request_id)
        if "error" in response:
            raise AssertionError(f"{method} failed: {response['error']}")
        return response["result"]

    def _receive(self, predicate: Any) -> dict[str, Any]:
        for index, message in enumerate(self._buffered):
            if predicate(message):
                return self._buffered.pop(index)
        while True:
            try:
                message = self._messages.get(timeout=8)
            except queue.Empty as error:
                stderr = ""
                if self._process.poll() is not None and self._process.stderr is not None:
                    stderr = self._process.stderr.read()
                raise AssertionError(
                    f"timed out waiting for Provider response; exit={self._process.poll()} stderr={stderr}"
                ) from error
            if predicate(message):
                return message
            self._buffered.append(message)

    def close(self) -> None:
        if self._process.stdin is not None and not self._process.stdin.closed:
            self._process.stdin.close()
        try:
            status = self._process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait(timeout=2)
            raise AssertionError("Provider did not exit after provider.shutdown")
        if status != 0:
            assert self._process.stderr is not None
            raise AssertionError(
                f"Provider exited with status {status}: {self._process.stderr.read()}"
            )

    def kill(self) -> None:
        if self._process.poll() is None:
            self._process.kill()
            self._process.wait(timeout=2)


def route() -> dict[str, str]:
    return {
        "deviceId": "python-black-box-device",
        "providerPluginId": "dev.codepet.codex",
        "providerInstanceId": "codex",
    }


def run(provider_executable: Path, app_server_executable: Path) -> None:
    provider = ProviderProcess(provider_executable)
    try:
        initialized = provider.request(
            "initialize",
            "provider.initialize",
            {
                "hostClientId": "python-black-box-client",
                "hostDeviceId": route()["deviceId"],
                "hostVersion": "test",
                "supportedVersions": {"minVersion": 1, "maxVersion": 1},
            },
        )
        assert initialized["selectedVersion"] == 1
        assert initialized["plugin"]["pluginId"] == "dev.codepet.codex"

        described = provider.request("describe", "provider.describe", {})
        assert described["plugin"] == initialized["plugin"]

        with tempfile.TemporaryDirectory(prefix="codepet-codex-provider-") as directory:
            marker = str(Path(directory) / "fixture-marker.txt")
            created = provider.request(
                "create-instance",
                "instance.create",
                {
                    "route": route(),
                    "instanceKind": "codex",
                    "displayName": "Codex Python Black Box",
                    "settings": {
                        "appServerExecutable": str(app_server_executable),
                        "appServerArgs": [
                            "--approval-mode",
                            "normal",
                            "--marker",
                            marker,
                        ],
                    },
                },
            )
            assert created["instance"]["status"] == "created"

            started = provider.request(
                "start-instance", "instance.start", {"route": route()}
            )
            assert started["instance"]["status"] == "ready"
            assert started["instance"]["harness"]["id"] == "codex"

            listed = provider.request(
                "list-conversations",
                "conversation.list",
                {"route": route(), "limit": 10},
            )
            assert isinstance(listed["conversations"], list)
            assert "pageInfo" in listed

            conversation = provider.request(
                "create-conversation",
                "conversation.create",
                {
                    "route": route(),
                    "permissionLevel": "workspace-write",
                    "model": "gpt-fixture",
                    "reasoningEffort": "high",
                    "workspaceRoot": "/fixture/workspace",
                },
            )["conversation"]
            fetched = provider.request(
                "get-conversation",
                "conversation.get",
                {"conversation": conversation["resource"]},
            )
            assert fetched["conversation"]["resource"] == conversation["resource"]
            assert isinstance(fetched["items"], list)

            stopped = provider.request(
                "stop-instance", "instance.stop", {"route": route()}
            )
            assert stopped["instance"]["status"] == "stopped"

        shutdown = provider.request("shutdown", "provider.shutdown", {})
        assert shutdown["accepted"] is True
        provider.close()
    except BaseException:
        provider.kill()
        raise


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", required=True, type=Path)
    parser.add_argument("--app-server", required=True, type=Path)
    arguments = parser.parse_args()
    run(arguments.provider.resolve(), arguments.app_server.resolve())
    print("Codex Provider stdio black-box smoke passed")


if __name__ == "__main__":
    main()
