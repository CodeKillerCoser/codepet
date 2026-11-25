import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import Request, urlopen
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import threading

from dataset import render_full_document
from engine import configure
from mlx_gateway import AuditedGateway, assess_calls, loopback_base
from render_report import knowledge_documents
from probe import make_update_fixture


class MLXProbeTests(unittest.TestCase):
    def test_update_fixture_changes_width_statements_without_corrupting_dates(self):
        original = (
            "2026/11/24 23:58:00；2026/11/25 00:03:36\n"
            "抽屉默认宽度改为窗口宽度的 1/2；范围仍为 1/3～3/4。\n"
            "抽屉默认宽度及双击复位已按用户要求改为窗口的 1/2；首次挂载 CSS 后备 50vw。\n"
            "其他比例 1/2 与其他 CSS 50vw 保持原样。"
        )
        before, after = make_update_fixture(original)
        self.assertTrue(before.startswith(original))
        self.assertIn("2026/11/24 23:58:00；2026/11/25 00:03:36", after)
        self.assertIn("范围仍为 1/3～3/4", after)
        self.assertIn("其他比例 1/2 与其他 CSS 50vw 保持原样", after)
        self.assertEqual(after.count("3/5"), 2)
        self.assertIn("首次挂载 CSS 后备 60vw", after)
        self.assertIn("ProbeNewLibrary", after)
        self.assertNotIn("ProbeOldLibrary", after)

    def test_knowledge_document_keeps_later_chunks_and_excludes_control_updates(self):
        def extraction(source, name, call):
            return {"source": source, "call": call, "nodes": [{"id": "n1", "name": name}], "edges": []}
        documents = [{"runId": "r", "taskTitle": "主题", "text": "首段\n\n后续历史"}]
        result = knowledge_documents(documents, [extraction("首段", "目标", "1"),
                                                extraction("后续历史", "后来决策", "2"),
                                                extraction("首段 fixture", "控制样本", "3"),
                                                extraction("后续历史", "后来决策", "4")])
        self.assertEqual(result[0]["calls"], ["1", "2"])
        self.assertIn("后来决策", result[0]["text"])
        self.assertNotIn("控制样本", result[0]["text"])

    def test_history_keeps_earlier_decision_and_separates_expected_validation(self):
        doc = {"runId": "r", "taskTitle": "抽屉", "taskDescription": "背景", "title": "修复",
               "status": "ready_for_delivery", "goal": "调宽", "solution": "拖动", "description": "",
               "activity": [{"at": 1, "kind": "workspace_snapshot", "summary": "机器扫描"},
                            {"at": 2, "kind": "milestone", "summary": "默认宽度 1/3"},
                            {"at": 3, "kind": "milestone", "summary": "改为 1/2，待验收"},
                            {"at": 3, "kind": "milestone", "summary": "改为 1/2，待验收"}],
               "criteria": [{"title": "调宽", "steps": "拖动", "expected": "范围正确"}]}
        text = render_full_document(doc)
        self.assertNotIn("机器扫描", text)
        self.assertLess(text.index("默认宽度 1/3"), text.index("改为 1/2"))
        self.assertEqual(text.count("改为 1/2"), 1)
        self.assertIn("预期验收条件（不是通过证据）", text)
        self.assertIn("ready_for_delivery", text)

    def test_nonlocal_or_credential_bearing_api_is_rejected(self):
        for base in ("https://api.openai.com/v1", "http://192.168.1.2/v1", "http://key@localhost/v1"):
            with self.assertRaises(ValueError):
                loopback_base(base)

    def test_mlx_configuration_pins_both_models_and_local_apis(self):
        with tempfile.TemporaryDirectory() as temp, patch.dict("os.environ", {}, clear=True):
            configure(Path(temp) / "run", Path(temp) / "cache", "Qwen3-Embedding-0.6B-8bit",
                      backend="mlx", endpoint="http://127.0.0.1:11234/v1")
            import os
            self.assertEqual(os.environ["GRAPH_EXTRACTOR"], "llm")
            self.assertEqual(os.environ["EMBEDDING_DIMENSIONS"], "1024")
            self.assertEqual(os.environ["LLM_ENDPOINT"], os.environ["EMBEDDING_ENDPOINT"])
            self.assertFalse(json.loads(os.environ["LLM_ARGS"])["extra_body"]["chat_template_kwargs"]["enable_thinking"])

    def test_adapter_forwards_embeddings_and_omits_vectors_and_credentials_from_evidence(self):
        class Fake(BaseHTTPRequestHandler):
            requests = []
            def log_message(self, *_args):
                pass

            def do_POST(self):
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                Fake.requests.append(payload)
                count = len(payload.get("input", [""]))
                body = json.dumps({"data": [{"index": i, "embedding": [0.6, 0.8]} for i in range(count)]}).encode()
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Fake)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as temp:
                gateway = AuditedGateway(f"http://127.0.0.1:{server.server_port}/v1", Path(temp))
                endpoint = gateway.start()
                try:
                    request = Request(endpoint + "/embeddings", data=b'{"input":["query"]}',
                                      headers={"Authorization": "Bearer user-secret", "Content-Type": "application/json"})
                    response = json.load(urlopen(request, timeout=5))
                    self.assertEqual(response["data"][0]["embedding"], [0.6, 0.8])
                    for kind in ("json_object", "json_schema"):
                        request = Request(endpoint + "/chat/completions",
                                          data=json.dumps({"response_format": {"type": kind}, "messages": []}).encode(),
                                          headers={"Content-Type": "application/json"})
                        json.load(urlopen(request, timeout=5))
                    self.assertNotIn("response_format", Fake.requests[1])
                    self.assertEqual(Fake.requests[2]["response_format"]["type"], "json_schema")
                    request = Request(endpoint + "/embeddings", data=json.dumps({"model": "test", "input": ["x"] * 17}).encode(),
                                      headers={"Content-Type": "application/json"})
                    merged = json.load(urlopen(request, timeout=5))
                    self.assertEqual([row["index"] for row in merged["data"]], list(range(17)))
                    self.assertEqual([len(item["input"]) for item in Fake.requests[3:]], [8, 8, 1])
                finally:
                    gateway.close()
                evidence = (Path(temp) / "0001.json").read_text()
                self.assertNotIn("user-secret", evidence)
                self.assertNotIn("0.6", evidence)
                self.assertEqual(json.loads(evidence)["response"]["dimensions"], [2])
                resumed = AuditedGateway(f"http://127.0.0.1:{server.server_port}/v1", Path(temp))
                try:
                    self.assertEqual(resumed.counter, 4)
                finally:
                    resumed.server.server_close()
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    def test_exact_request_reuses_saved_model_reply_without_upstream_inference(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            reuse = root / "reuse"
            reuse.mkdir()
            answer = {"choices": [{"message": {"content": '{"nodes":[],"edges":[]}'}}]}
            saved = {"route": "chat/completions", "status": 200,
                     "request": {"messages": [], "max_tokens": 4096}, "response": answer}
            (reuse / "0001.json").write_text(json.dumps(saved))
            gateway = AuditedGateway("http://127.0.0.1:1/v1", root / "new", reuse=reuse)
            try:
                request = Request(gateway.start() + "/chat/completions",
                                  data=b'{"messages":[],"response_format":{"type":"json_object"}}',
                                  headers={"Content-Type": "application/json"})
                self.assertEqual(json.load(urlopen(request, timeout=5)), answer)
                evidence = json.loads((root / "new/0001.json").read_text())
                self.assertEqual(evidence["reusedFrom"], str((reuse / "0001.json").resolve()))
            finally:
                gateway.close()

    def test_evidence_checker_flags_invented_quote_without_claiming_entailment(self):
        with tempfile.TemporaryDirectory() as temp:
            graph = {"nodes": [], "edges": [{"description": "证据=「原文」"},
                                             {"description": "证据=「不存在的证据」"}]}
            call = {"route": "chat/completions", "seconds": 1,
                    "request": {"messages": [{"role": "user", "content": "这是原文。"}]},
                    "response": {"choices": [{"message": {"content": json.dumps(graph)}}]}}
            (Path(temp) / "0001.json").write_text(json.dumps(call))
            assessment = assess_calls(Path(temp))
            self.assertEqual(len(assessment["extractions"]), 1)
            self.assertEqual(len(assessment["missingOrNonliteralEvidence"]), 1)
            self.assertIn("do not establish factual entailment", assessment["note"])


if __name__ == "__main__":
    unittest.main()
