#!/usr/bin/env python3
"""Run the Rust public-wire smoke against selected Codex Provider/fixture binaries.

The test client negotiates the SDK mux protocol before sending JSON-RPC requests.
Python is only the command wrapper; it does not implement a second mux runtime.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", required=True, type=Path)
    parser.add_argument("--app-server", required=True, type=Path)
    arguments = parser.parse_args()
    environment = os.environ.copy()
    environment["CODEPET_TEST_PROVIDER_EXE"] = str(arguments.provider.resolve(strict=True))
    environment["CODEPET_TEST_APP_SERVER_EXE"] = str(arguments.app_server.resolve(strict=True))
    repository = Path(__file__).resolve().parent.parent
    subprocess.run([
        environment.get("CARGO", "cargo"), "test", "--manifest-path", str(repository / "crates/Cargo.toml"),
        "-p", "codepet-provider-codex", "--test", "provider_vertical",
        "provider_binary_public_mux_smoke", "--", "--exact",
    ], cwd=repository, env=environment, check=True)
    print("Codex Provider mux stdio black-box smoke passed")


if __name__ == "__main__":
    main()
