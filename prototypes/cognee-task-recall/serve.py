"""Serve the latest local evidence bundle; no connection to the production MCP."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-root", type=Path, default=Path(__file__).resolve().parents[2] / "artifacts/cognee-task-recall-qa")
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    root = args.output_root.resolve()
    output = Path(json.loads((root / "latest.json").read_text())["output"]).resolve()
    if root not in output.parents or not (output / "index.html").exists():
        parser.error("Latest report must exist beneath --output-root")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), partial(SimpleHTTPRequestHandler, directory=str(output)))
    print(f"http://127.0.0.1:{server.server_port}/", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        server.server_close()
