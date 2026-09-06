"""Run a real Claude -p hook smoke with temporary settings and a local receiver."""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    args = parser.parse_args()
    events = []

    class Receiver(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            events.append(payload)
            print(json.dumps(payload, ensure_ascii=False), flush=True)
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Receiver)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="codepet claude hook ") as directory:
            root = Path(directory)
            hook = root / "receive.py"
            hook.write_text(
                "import sys,urllib.request\n"
                "data=sys.stdin.buffer.read()\n"
                "urllib.request.urlopen(urllib.request.Request(sys.argv[1],data=data,"
                "headers={'Content-Type':'application/json'}),timeout=3).read()\n",
                encoding="utf-8",
            )
            handler = {"type": "command", "command": sys.executable,
                       "args": [str(hook), f"http://127.0.0.1:{server.server_port}/"], "timeout": 10}
            config = root / "settings.json"
            config.write_text(json.dumps({"hooks": {
                event: [{"hooks": [handler]}] for event in ["SessionStart", "UserPromptSubmit", "Stop"]
            }}), encoding="utf-8")
            result = subprocess.run(
                [str(args.runner.resolve()), str(args.executable.resolve()), "-p", "Reply exactly OK. Do not use tools.",
                 "--settings", str(config), "--output-format", "json", "--max-turns", "1"],
                cwd=root, capture_output=True, text=True, encoding="utf-8", errors="replace",
                timeout=180, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
            )
            print("Claude exit:", result.returncode, flush=True)
            if result.returncode:
                print(result.stderr[-2000:], flush=True)
            received = {event.get("hook_event_name") for event in events}
            print("Received events:", sorted(received), flush=True)
            assert result.returncode == 0, "Claude -p failed"
            assert {"UserPromptSubmit", "Stop"} <= received, "Claude did not invoke the configured hooks"
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
