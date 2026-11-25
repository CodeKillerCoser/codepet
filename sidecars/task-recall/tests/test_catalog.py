import hashlib
import tempfile
import unittest
import uuid
import asyncio
import threading
from pathlib import Path
from codepet_recall.catalog import Catalog
from codepet_recall.server import Service


def document(text="抽屉预览宽度恢复默认", **overrides):
    return {"runId":str(uuid.uuid4()), "taskId":str(uuid.uuid4()), "text":text,
            "contentHash":hashlib.sha256(text.encode()).hexdigest(), "eligible":True, **overrides}


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.catalog = Catalog(self.root)

    def tearDown(self):
        self.catalog.db.close()
        self.temp.cleanup()

    def test_chinese_fulltext_and_unchanged_sync(self):
        doc = document()
        self.catalog.sync([doc])
        row = self.catalog.claim()
        self.catalog.finish(row, "data-1")
        self.catalog.sync([doc])
        self.assertIsNone(self.catalog.claim())
        self.assertEqual(self.catalog.lexical("抽屉宽度")[0]["runId"], doc["runId"])
        self.assertEqual(self.catalog.lexical("银行对账"), [])

    def test_superseded_inflight_version_never_ready(self):
        doc = document()
        self.catalog.sync([doc])
        row = self.catalog.claim()
        updated = document("改为全局窗口栈", runId=doc["runId"], taskId=doc["taskId"])
        self.catalog.sync([updated])
        self.catalog.finish(row, "data-1")
        self.assertEqual(self.catalog.ready(), {})
        next_row = self.catalog.claim()
        self.assertEqual(next_row["data_id"], "data-1")
        self.catalog.finish(next_row, "data-1")
        self.assertEqual(self.catalog.ready()[doc["runId"]]["text"], updated["text"])

    def test_restart_recovers_processing_and_retries_failure(self):
        self.catalog.sync([document()])
        self.catalog.claim()
        self.catalog.db.close()
        self.catalog = Catalog(self.root)
        row = self.catalog.claim()
        self.catalog.finish(row, error="Offline")
        self.assertIsNone(self.catalog.claim())
        self.assertEqual(self.catalog.status()["counts"]["failed"], 1)
        self.assertEqual(self.catalog.ready(), {})

    def test_deletion_and_draft_block_stale_results(self):
        doc = document()
        self.catalog.sync([doc]); row = self.catalog.claim()
        self.catalog.sync([{**doc,"eligible":False}])
        self.catalog.finish(row, "data-1")
        self.assertEqual(self.catalog.ready(), {})
        self.assertIsNone(self.catalog.claim())
        self.catalog.sync([])
        self.assertEqual(self.catalog.lexical("抽屉"), [])
        row = self.catalog.claim()
        self.catalog.sync([])  # periodic sync must not reset in-flight deletion
        self.assertIsNone(self.catalog.claim())
        self.catalog.finish(row)
        self.assertEqual(self.catalog.db.execute("SELECT count(*) FROM runs").fetchone()[0], 0)

    def test_invalid_sync_cannot_delete_existing_documents(self):
        doc = document()
        self.catalog.sync([doc])
        with self.assertRaises(ValueError):
            self.catalog.sync([{**doc,"text":"corrupt"}])
        self.assertEqual(len(self.catalog.lexical("抽屉")), 1)

    def test_model_initialization_failure_keeps_fulltext_available(self):
        doc = document()
        self.catalog.sync([doc])
        service = Service.__new__(Service)
        service.catalog = self.catalog
        service.config = {"enabled":True}
        service.busy = threading.Lock()
        service.loop = asyncio.new_event_loop()
        class Offline:
            async def search(self, *_):
                raise RuntimeError("upstream unavailable")
        service.engine = Offline()
        thread = threading.Thread(target=service.loop.run_forever, daemon=True)
        thread.start()
        try:
            result = service.query({"query":"抽屉"})
            self.assertEqual(result["hits"][0]["runId"],doc["runId"])
            self.assertEqual(result["recall"]["state"],"degraded")
        finally:
            service.loop.call_soon_threadsafe(service.loop.stop)
            thread.join(timeout=2)
            service.loop.close()


if __name__ == "__main__":
    unittest.main()
