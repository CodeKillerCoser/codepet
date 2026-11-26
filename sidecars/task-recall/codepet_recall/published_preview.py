"""Last successful preview, separate from the writer's disposable live cache."""
import json
from pathlib import Path
import threading
from .graph_cache import GraphCache


class PublishedPreview:
    def __init__(self, root):
        self.root = Path(root)
        self.cache = GraphCache(self.root / "published")
        self.lock = threading.RLock()

    def get(self, semantic=False):
        with self.lock:
            try:
                dataset = json.loads((self.root / "dataset.json").read_text())["id"]
            except (OSError, ValueError, KeyError, TypeError):
                return None
            base = self.cache.get(dataset, False)
            if base is None:
                return None
            projected = self.cache.get(dataset, True) if semantic else None
            if projected and projected["previewVersion"] == base["previewVersion"]:
                return projected
            return base

    def publish(self, dataset, graph, projected=None):
        if not graph:
            return
        with self.lock:
            self.cache.put(dataset, False, graph)
            if projected and projected["previewVersion"] == graph["previewVersion"]:
                self.cache.put(dataset, True, projected)
