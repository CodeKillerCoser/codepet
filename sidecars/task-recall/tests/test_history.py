import asyncio
import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
import uuid
from codepet_recall.catalog import Catalog
from codepet_recall.history import QueueConflict, read_history
from codepet_recall.results import processing_result


def document(text="任务摘要", **updates):
    return {"runId":str(uuid.uuid4()), "taskId":str(uuid.uuid4()), "taskTitle":"长期主题",
        "summary":{"title":"本轮执行"}, "text":text, "contentHash":hashlib.sha256(text.encode()).hexdigest(),
        "eligible":True, **updates}


class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.catalog = Catalog(self.root / "current")
        self.doc = document()
        self.catalog.sync([self.doc])

    def tearDown(self):
        self.catalog.db.close()
        self.temp.cleanup()

    def items(self):
        return read_history(self.root, "current")["items"]

    def test_cancel_survives_unchanged_sync_restart_and_new_version_is_distinct(self):
        job = self.items()[0]
        self.catalog.cancel(job["id"])
        self.catalog.sync([self.doc])
        self.catalog.db.close()
        self.catalog = Catalog(self.root / "current")
        self.catalog.sync([self.doc])
        self.assertIsNone(self.catalog.claim())
        self.assertEqual(self.items()[0]["state"], "cancelled")
        self.assertTrue(self.catalog.lexical("任务"))
        updated = document("新版本摘要", runId=self.doc["runId"], taskId=self.doc["taskId"])
        self.catalog.sync([updated])
        self.assertEqual({row["state"] for row in self.items()}, {"queued", "cancelled"})
        self.assertNotEqual(self.catalog.claim()["job_id"], job["id"])

    def test_claim_wins_cancellation_race_without_killing_processing(self):
        job = self.items()[0]
        row = self.catalog.claim()
        with self.assertRaises(QueueConflict):
            self.catalog.cancel(job["id"])
        self.assertEqual(self.items()[0]["state"], "processing")
        self.catalog.finish(row, "data", result={"description":"原生结果", "summaries":["原生摘要"], "entities":[]})
        detail = read_history(self.root, "current", job=job["id"])
        self.assertEqual(detail["result"]["summaries"], ["原生摘要"])
        self.assertIsNotNone(detail["finishedAt"])
        self.assertGreaterEqual(detail["elapsedMs"], 0)
        self.assertTrue(detail["usageTracked"])

    def test_late_old_completion_cannot_resurrect_cancelled_new_version(self):
        old = self.catalog.claim()
        updated = document("新的方案", runId=self.doc["runId"], taskId=self.doc["taskId"])
        self.catalog.sync([updated])
        queued = next(row for row in self.items() if row["state"] == "queued")
        self.catalog.cancel(queued["id"])
        self.catalog.finish(old, "native-data")
        self.assertIsNone(self.catalog.claim())
        self.assertEqual({row["state"] for row in self.items()}, {"superseded", "cancelled"})
        with self.assertRaises(QueueConflict):
            self.catalog.retry(old["job_id"])

    def test_failure_preserves_attempt_and_retry_queue_can_be_cancelled(self):
        row = self.catalog.claim()
        self.catalog.finish(row, error="TimeoutError")
        rows = self.items()
        self.assertEqual({r["state"] for r in rows}, {"failed", "queued"})
        queued = next(r for r in rows if r["state"] == "queued")
        self.catalog.cancel(queued["id"])
        self.catalog.db.execute("UPDATE runs SET retry_at=0"); self.catalog.db.commit()
        self.assertIsNone(self.catalog.claim())
        new = self.catalog.retry(row["job_id"])
        self.assertNotEqual(new["id"], row["job_id"])
        self.assertEqual(self.catalog.claim()["job_id"], new["id"])

    def test_recovery_keeps_interrupted_attempt_and_stale_actions_fail(self):
        old = self.catalog.claim()
        self.catalog.db.close()
        self.catalog = Catalog(self.root / "current")
        self.assertEqual({r["state"] for r in self.items()}, {"queued", "interrupted"})
        with self.assertRaises(QueueConflict):
            self.catalog.cancel(old["job_id"])
        self.assertNotEqual(self.catalog.claim()["job_id"], old["job_id"])

    def test_legacy_complete_has_unknown_time_and_tokens_without_reprocessing(self):
        self.catalog.finish(self.catalog.claim(), "data")
        self.catalog.db.execute("DELETE FROM processing_jobs")
        self.catalog.db.execute("UPDATE runs SET job_id=NULL")
        self.catalog.db.commit(); self.catalog.db.close()
        self.catalog = Catalog(self.root / "current")
        job = self.items()[0]
        self.assertEqual(job["state"], "completed")
        self.assertFalse(job["usageTracked"])
        self.assertIsNone(job["finishedAt"])
        self.assertIsNone(job["elapsedMs"])
        self.assertIsNone(self.catalog.claim())

    def test_manual_task_groups_finished_runs_and_reuses_pending_work(self):
        other = document("另一执行", taskId=self.doc["taskId"])
        draft = document("开发中", taskId=self.doc["taskId"], eligible=False)
        unrelated = document("另一个任务")
        self.catalog.finish(self.catalog.claim(), "old-data")
        self.catalog.sync([self.doc, other, draft, unrelated])
        before = len(self.items())
        result = self.catalog.enqueue_task(self.doc["taskId"], str(uuid.uuid4()))
        self.assertEqual(result["queued"], 1)
        self.assertEqual(result["existing"], 1)
        self.assertEqual(len(self.items()), before + 1)
        self.assertEqual({row["runId"] for row in self.items() if row["id"] in result["jobIds"]}, {self.doc["runId"], other["runId"]})
        self.assertFalse(any(row["runId"] == draft["runId"] for row in self.items()))

    def test_manual_receipt_is_idempotent_even_after_completion_and_restart(self):
        self.catalog.finish(self.catalog.claim(), "existing-data")
        operation = str(uuid.uuid4())
        receipt = self.catalog.enqueue_task(self.doc["taskId"], operation)
        row = self.catalog.claim()
        self.assertTrue(row["rebuild"])
        self.catalog.finish(row, "existing-data")
        self.catalog.db.close(); self.catalog = Catalog(self.root / "current")
        self.assertEqual(self.catalog.enqueue_task(self.doc["taskId"], operation), receipt)
        self.assertIsNone(self.catalog.claim())
        self.assertEqual(len(self.items()), 2)
        with self.assertRaises(QueueConflict):
            self.catalog.enqueue_task(str(uuid.uuid4()), operation)

    def test_manual_rebuild_survives_retry_restart_and_cancellation(self):
        self.catalog.finish(self.catalog.claim(), "existing-data")
        result = self.catalog.enqueue_task(self.doc["taskId"], str(uuid.uuid4()))
        self.catalog.cancel(result["jobIds"][0])
        self.catalog.sync([self.doc])
        self.assertIsNone(self.catalog.claim())
        self.catalog.enqueue_task(self.doc["taskId"], str(uuid.uuid4()))
        first = self.catalog.claim()
        self.catalog.finish(first, error="TimeoutError")
        self.catalog.enqueue_task(self.doc["taskId"], str(uuid.uuid4()))
        self.assertTrue(self.catalog.claim()["rebuild"])
        self.catalog.db.close(); self.catalog = Catalog(self.root / "current")
        recovered = self.catalog.claim()
        self.assertTrue(recovered["rebuild"])
        self.catalog.finish(recovered, "existing-data")
        self.assertEqual(self.catalog.db.execute("SELECT rebuild FROM runs").fetchone()[0], 0)

    def test_manual_rejects_invalid_missing_and_developing_tasks(self):
        with self.assertRaises(ValueError):
            self.catalog.enqueue_task("invalid", str(uuid.uuid4()))
        with self.assertRaises(QueueConflict):
            self.catalog.enqueue_task(str(uuid.uuid4()), str(uuid.uuid4()))
        self.catalog.sync([{**self.doc, "eligible":False}])
        with self.assertRaises(QueueConflict):
            self.catalog.enqueue_task(self.doc["taskId"], str(uuid.uuid4()))

    def test_cross_configuration_history_is_paginated_and_inactive_queue_paused(self):
        other = Catalog(self.root / "previous")
        other.sync([document("旧配置")]); other.claim(); other.db.close()
        result = read_history(self.root, "current", limit=1)
        self.assertEqual(result["total"], 2)
        self.assertTrue(result["hasMore"])
        paused = read_history(self.root, "current", state="paused")["items"]
        self.assertEqual(len(paused), 1)
        self.assertFalse(paused[0]["canCancel"])
        self.assertIsNone(paused[0]["elapsedMs"])
        self.assertNotIn("document", result["items"][0])

    def test_native_result_provenance_ignores_text_mentions_and_unrelated_entities(self):
        meta = {"runId":self.doc["runId"], "contentHash":self.doc["contentHash"]}
        graph = {"nodes":[
            {"id":"chunk", "type":"DocumentChunk", "external_metadata":json.dumps(meta)},
            {"id":"entity", "type":"Entity", "name":"所属实体"},
            {"id":"summary", "type":"TextSummary", "source_chunk_id":"chunk", "text":"原生摘要"},
            {"id":"other", "type":"Entity", "name":self.doc["runId"]},
            {"id":"old", "type":"DocumentChunk", "external_metadata":{**meta,"contentHash":"old-version"}},
        ], "links":[{"source":"chunk", "target":"entity"}, {"source":"old", "target":"other"}]}
        result = processing_result(graph, self.doc)
        self.assertEqual(result["summaries"], ["原生摘要"])
        self.assertEqual([row["name"] for row in result["entities"]], ["所属实体"])
