"""Disposable native preview snapshots; ingestion invalidates before touching the index."""
import json
import os
from pathlib import Path
import tempfile


class GraphCache:
    VERSION = "cognee-1.6.10-preview-1"
    MAX_BYTES = 32 * 1024 * 1024

    def __init__(self, root):
        self.root = Path(root)
        self.memory = {}

    def path(self, semantic):
        return self.root / ("preview-semantic.json" if semantic else "preview-graph.json")

    def get(self, dataset, semantic):
        key = (dataset, semantic)
        if key in self.memory:
            return self.memory[key]
        path = self.path(semantic)
        try:
            if path.stat().st_size > self.MAX_BYTES:
                return None
            saved = json.loads(path.read_text())
            graph = saved["graph"]
            if (saved["version"] == self.VERSION and saved["dataset"] == dataset
                    and isinstance(graph.get("nodes"), list) and isinstance(graph.get("links"), list)
                    and isinstance(graph.get("previewVersion"), str)):
                self.memory[key] = graph
                return graph
        except (OSError, ValueError, KeyError, TypeError, AttributeError):
            pass
        return None

    def put(self, dataset, semantic, graph):
        self.memory[(dataset, semantic)] = graph
        # A cache failure must not turn a successful read into a failed model attempt.
        temporary = None
        try:
            raw = json.dumps({"version":self.VERSION, "dataset":dataset, "graph":graph})
            if len(raw.encode()) > self.MAX_BYTES:
                return
            self.root.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=self.root, delete=False) as file:
                temporary = Path(file.name)
                file.write(raw)
            os.replace(temporary, self.path(semantic))
        except OSError:
            pass
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)

    def invalidate(self):
        self.memory.clear()
        # If an old disk snapshot cannot be removed, fail before changing native data.
        for semantic in (False, True):
            self.path(semantic).unlink(missing_ok=True)
