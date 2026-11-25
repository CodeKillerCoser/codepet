"""Meaningful boundary checks: read-only export, provenance, and history noise."""

import json
import tempfile
import unittest
import subprocess
import sys
from pathlib import Path

from dataset import build_queries, read_ledger, source_is_unchanged
from engine import candidate_ids, fuse, local_file, ownership_map


class SnapshotTests(unittest.TestCase):
    def test_exports_each_run_without_overwriting_topic_or_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / "projects/p/tasks/t.json"
            target.parent.mkdir(parents=True)
            runs = []
            for index in range(2):
                runs.append({
                    "id": f"r{index}", "title": f"处理 {index}", "kind": "feature",
                    "goal": f"目标 {index}", "solution": "确定方案", "description": "",
                    "status": "in_progress", "createdAt": index, "updatedAt": index + 1,
                    "agentWorkspace": {"path": f"/worktree/{index}"},
                    "timeline": [
                        {"kind": "milestone", "summary": "重复进度"},
                        {"kind": "milestone", "summary": "重复进度"},
                        {"kind": "milestone", "summary": "最新结论"},
                    ],
                })
            original = json.dumps({
                "schemaVersion": 4, "id": "t", "title": "主题", "description": "背景",
                "projectId": "p", "revision": 9, "runs": runs,
            }).encode()
            target.write_bytes(original)
            documents, hashes = read_ledger(root)
            self.assertEqual([doc["runId"] for doc in documents], ["r0", "r1"])
            self.assertEqual([doc["isCurrentRun"] for doc in documents], [False, True])
            self.assertEqual(documents[0]["workspace"], "/worktree/0")
            self.assertEqual(documents[0]["sourceRevision"], 9)
            self.assertEqual(documents[0]["activityCount"], 3)
            self.assertNotIn("重复进度", documents[0]["text"])
            self.assertIn("最新结论", documents[0]["text"])
            self.assertEqual(target.read_bytes(), original)
            self.assertTrue(source_is_unchanged(hashes))
            target.write_bytes(original + b" ")
            self.assertFalse(source_is_unchanged(hashes))

    def test_benchmark_keeps_negative_query_and_source_annotated_targets(self):
        queries = build_queries([{"runId": "r", "taskTitle": "任务 MCP 改为本机 HTTP"}])
        self.assertEqual(queries[0]["expectedRunIds"], ["r"])
        self.assertEqual(queries[-1]["expectedRunIds"], [])


class RecallBoundaryTests(unittest.TestCase):
    def test_real_write_and_rename_are_denied_while_artifact_writes_work(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            ledger = root / "ledger"
            ledger.mkdir()
            source = ledger / "task.json"
            source.write_text("original")
            script = """
import json, os, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from readonly import ReadOnlyLedger
root = Path(sys.argv[2])
guard = ReadOnlyLedger(root / 'ledger')
guard.install()
blocked = []
for operation in [lambda: (root/'ledger/task.json').write_text('bad'),
                  lambda: os.rename(root/'ledger/task.json', root/'moved.json')]:
    try:
        operation()
    except PermissionError:
        blocked.append(True)
(root/'artifact.json').write_text('allowed')
print(json.dumps({'blocked': blocked, 'attempts': guard.blocked_writes}))
"""
            result = subprocess.run([sys.executable, "-c", script, str(Path(__file__).parent), str(root)],
                                    capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(result.stdout), {"blocked": [True, True], "attempts": 2})
            self.assertEqual(source.read_text(), "original")
            self.assertEqual((root / "artifact.json").read_text(), "allowed")

    def test_metadata_maps_a_chunk_without_a_run_id_in_its_text(self):
        run_id = "11111111-1111-4111-8111-111111111111"
        chunk_id = "22222222-2222-4222-8222-222222222222"
        other_id = "33333333-3333-4333-8333-333333333333"
        rows = [{"objects_result": [{"id": chunk_id, "payload": {"text": "目标段落"}}],
                 "context_result": other_id, "evidence": []}]
        self.assertEqual(candidate_ids(rows, [{"runId": run_id}, {"runId": other_id}],
                                       {chunk_id: [run_id]}), [run_id])

    def test_graph_ownership_does_not_spread_through_semantic_relations(self):
        graph = {"nodes": [
            {"id": "c", "type": "DocumentChunk", "external_metadata": '{"runId":"r"}'},
            {"id": "a", "type": "Entity"}, {"id": "b", "type": "Entity"}],
            "links": [{"source": "c", "target": "a", "relation": "contains"},
                      {"source": "a", "target": "b", "relation": "depends_on"}]}
        owners = ownership_map(graph, {})
        self.assertEqual(owners["a"], ["r"])
        self.assertNotIn("b", owners)

    def test_workspace_does_not_exclude_a_matching_run_elsewhere(self):
        documents = [{"runId": "a", "workspace": "/current", "updatedAt": 2},
                     {"runId": "b", "workspace": "/other", "updatedAt": 1}]
        result = fuse({"semantic": {"runIds": ["b"]}}, documents, "/current")
        self.assertEqual([row["runId"] for row in result], ["b"])

    def test_cognee_file_uri_is_decoded_without_treating_it_as_a_relative_path(self):
        self.assertEqual(str(local_file("file:///tmp/probe%20copy.txt")), "/tmp/probe copy.txt")


if __name__ == "__main__":
    unittest.main()
