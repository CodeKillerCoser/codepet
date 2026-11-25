"""Run the upstream Rust processing APIs against the fixed raw Run snapshot."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from dataset import build_queries
from engine import MLX_EMBEDDER, MLX_LLM, candidate_ids, fuse, ownership_map, save_json
from mlx_gateway import AuditedGateway
from probe import graph_counts, make_update_fixture, summarize_metrics


class Driver:
    def __init__(self, binary: Path, output: Path, endpoint: str):
        self.output = output
        self.counter = max((int(p.name.split("-")[0]) for p in (output / "operations").glob("*.json")), default=0)
        runtime = output / "runtime"
        runtime.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ)
        env.update({
            "COGNEE_DISABLE_DOTENV": "1", "TELEMETRY_DISABLED": "1",
            "COGNEE_SYSTEM_ROOT_DIRECTORY": str(runtime / "system"),
            "COGNEE_DATA_ROOT_DIRECTORY": str(runtime / "data"),
            "CACHE_ROOT_DIRECTORY": str(runtime / "cache"),
            "DATABASE_URL": f"sqlite:{runtime / 'cognee.db'}?mode=rwc",
            "DB_PROVIDER": "sqlite", "GRAPH_DATABASE_PROVIDER": "ladybug",
            "VECTOR_DB_PROVIDER": "lancedb", "ENABLE_BACKEND_ACCESS_CONTROL": "false",
            "LLM_PROVIDER": "openai", "LLM_MODEL": MLX_LLM,
            "LLM_ENDPOINT": endpoint, "LLM_API_KEY": "mlx-local",
            "LLM_STRUCTURED_OUTPUT_MODE": "json", "LLM_TEMPERATURE": "0",
            "LLM_MAX_COMPLETION_TOKENS": "4096", "LLM_MAX_PARALLEL_REQUESTS": "1",
            "LLM_REQUEST_TIMEOUT_SECONDS": "600", "LLM_MIN_RETRY_SECONDS": "0",
            "LLM_ARGS": json.dumps({"chat_template_kwargs": {"enable_thinking": False}}),
            "EMBEDDING_PROVIDER": "openai_compatible", "EMBEDDING_MODEL": MLX_EMBEDDER,
            "EMBEDDING_ENDPOINT": endpoint, "EMBEDDING_API_KEY": "mlx-local",
            "EMBEDDING_DIMENSIONS": "1024", "EMBEDDING_MAX_COMPLETION_TOKENS": "4096",
            "EMBEDDING_BATCH_SIZE": "8", "COGNEE_CHUNK_SIZE": "4096",
            "COGNEE_CHUNK_OVERLAP": "10", "CHUNKS_PER_BATCH": "1",
            "GRAPH_PROMPT_PATH": str(Path(__file__).resolve().parent.parent / "development_graph_prompt.txt"),
        })
        self.log = (output / "driver.log").open("a")
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.log, env=env, cwd=runtime, text=True, bufsize=1)

    def call(self, op: str, **args):
        self.counter += 1
        command = {"op": op, **args}
        self.process.stdin.write(json.dumps(command, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError(f"Rust driver exited ({self.process.poll()}), see driver.log")
            try:
                response = json.loads(line)
            except json.JSONDecodeError:
                self.log.write(line)
                continue
            if isinstance(response, dict) and "ok" in response:
                break
        save_json(self.output / "operations" / f"{self.counter:03d}-{op}.json",
                  {"request": command, "response": response})
        if not response["ok"]:
            raise RuntimeError(response["error"])
        return response

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            self.process.wait(timeout=60)
        self.log.close()


def rows_by_run(driver):
    value = driver.call("rows")["result"]
    mapping = {}
    for row in value["rows"]:
        metadata = row.get("external_metadata")
        if isinstance(metadata, str):
            metadata = json.loads(metadata)
        if metadata and metadata.get("runId"):
            mapping[row["id"]] = metadata["runId"]
    return value, mapping


def snapshot(driver, output, name):
    value = driver.call("snapshot", html=str(output / f"{name}.html"))["result"]
    save_json(output / f"{name}.json", value["payload"])
    save_json(output / f"{name}-raw.json", value["raw"])
    return value["payload"]


def chunk_ids(graph):
    return {node["id"] for node in graph["nodes"] if node.get("type") == "DocumentChunk"}


def node_names(graph):
    return {str(node.get("name", "")).casefold(): node["id"]
            for node in graph["nodes"] if node.get("type") == "Entity"}


def comparison_fuse(routes, documents, workspace):
    """Keep framework-native hybrid outside the fixed three-route baseline."""
    return fuse({k:routes[k] for k in ("semantic", "lexical", "graph") if k in routes},
                documents, workspace)


def evaluate(driver, output, documents, report):
    started = time.perf_counter()
    print("Rust ingestion started", flush=True)
    if not report.get("ingestion"):
        report["ingestion"] = driver.call("ingest", documents=documents)
    save_json(output / "report.json", report)
    rows, mapping = rows_by_run(driver)
    report["documentRows"] = rows
    graph = snapshot(driver, output, "graph-baseline")
    report["graph"] = graph_counts(graph)
    owners = ownership_map(graph, mapping)
    # Chunk document_id is explicit source attribution, not semantic-edge propagation.
    raw = json.loads((output / "graph-baseline-raw.json").read_text())
    for node_id, props in raw["nodes"]:
        data_id = props.get("document_id") or props.get("is_part_of")
        if data_id in mapping:
            owners[node_id] = [mapping[data_id]]
    types = {node["id"]: node.get("type") for node in graph["nodes"]}
    for edge in graph["links"]:
        source, target = str(edge["source"]), str(edge["target"])
        if types.get(source) == "DocumentChunk" and types.get(target) in {"Entity", "TextSummary"}:
            for run_id in owners.get(source, []):
                if run_id not in owners.setdefault(target, []):
                    owners[target].append(run_id)
    save_json(output / "source-ownership.json", owners)
    report["checks"] = {"allRunsMapped": set(mapping.values()) == {d["runId"] for d in documents}}
    report["queries"] = []
    chat_before = len(list((output / "model-calls").glob("*.json")))
    for case in build_queries(documents):
        print("Rust query", case["id"], flush=True)
        routes = {}
        for name, kind in (("semantic", "CHUNKS"), ("lexical", "CHUNKS_LEXICAL"),
                           ("graph", "GRAPH_COMPLETION"), ("nativeHybrid", "HYBRID_COMPLETION")):
            try:
                result = driver.call("search", query=case["query"], type=kind)
                response = result["result"]
                contexts = list((response.get("context") or {}).values())
                objects = response.get("result", {}).get("data")
                if isinstance(objects, list) and name in {"semantic", "lexical"}:
                    if name == "lexical":
                        objects = [x for x in objects if isinstance(x, dict) and (x.get("score") or 0) > 0]
                    ranked = [{"objects_result": objects, "evidence": []}]
                else:
                    ranked = [{"objects_result": items, "evidence": []} for items in contexts]
                    ranked.append({"objects_result": [], "evidence": response.get("evidence", [])})
                ids = candidate_ids(ranked, documents, owners)
                routes[name] = {"runIds": ids, "raw": response, "seconds": result["seconds"]}
            except Exception as error:
                routes[name] = {"runIds": [], "error": str(error)}
        case["routes"] = routes
        case["fused"] = comparison_fuse(routes, documents, report["queryWorkspace"])
        report["queries"].append(case)
        save_json(output / "report.json", report)
    report["metrics"] = {lane: summarize_metrics(report["queries"], lane)
                         for lane in ("semantic", "lexical", "graph", "nativeHybrid", "fused")}
    calls = [json.loads(p.read_text()) for p in sorted((output / "model-calls").glob("*.json"))]
    report["checks"]["recallDoesNotCallLLM"] = not any(c["route"] == "chat/completions" for c in calls[chat_before:])
    report["checks"]["allRecallRoutesSucceeded"] = all(not x.get("error") for q in report["queries"] for x in q["routes"].values())
    target = next(d for d in documents if "统一全局侧栏" in d["taskTitle"])
    data_id = next(k for k, v in mapping.items() if v == target["runId"])
    before, after = make_update_fixture(target["text"])
    (output / "update-input-before.md").write_text(before)
    (output / "update-input-after.md").write_text(after)
    print("Rust update started", flush=True)
    changes = {}
    for label, text in (("before", before), ("after", after), ("duplicate", after), ("restored", target["text"])):
        doc = {**target, "text": text, "contentHash": hashlib.sha256(text.encode()).hexdigest()}
        changes[label] = driver.call("update", dataId=data_id, document=doc)
        rows_after, _ = rows_by_run(driver)
        changes[label]["documentRows"] = rows_after
        current = snapshot(driver, output, "update-graph-" + label)
        changes[label]["chunkIds"] = sorted(chunk_ids(current))
        changes[label]["names"] = node_names(current)
        changes[label]["vectorIds"] = [r["id"] for r in driver.call("vectors", ids=changes[label]["chunkIds"])["result"]]
        save_json(output / "report.json", {**report, "update": changes})
    before_graph = json.loads((output / "update-graph-before.json").read_text())
    after_graph = json.loads((output / "update-graph-after.json").read_text())
    after_text = json.dumps(after_graph, ensure_ascii=False)
    stale_chunks = set(changes["before"]["chunkIds"]) - set(changes["after"]["chunkIds"])
    stale_vectors = driver.call("vectors", ids=sorted(stale_chunks))["result"]
    checks = {
        "stableDataId": all(c["result"]["dataId"] == data_id for c in changes.values()),
        "oldTextRemoved": "CODEPET_PROBE_OLD_REVISION" not in after_text,
        "newTextPresent": "CODEPET_PROBE_NEW_REVISION" in after_text,
        "oldChunkVectorsRemoved": not stale_vectors if stale_chunks else None,
        "newChunkVectorsPresent": set(changes["after"]["chunkIds"]) == set(changes["after"]["vectorIds"]),
        "oldEntityExtracted": "probeoldlibrary" in changes["before"]["names"],
        "oldEntityRemoved": "probeoldlibrary" not in changes["after"]["names"],
        "newEntityPresent": "probenewlibrary" in changes["after"]["names"],
        "sharedEntityIdStable": changes["before"]["names"].get("probesharedlibrary") is not None and
            changes["before"]["names"].get("probesharedlibrary") == changes["after"]["names"].get("probesharedlibrary"),
        "duplicateChunkIdsStable": changes["after"]["chunkIds"] == changes["duplicate"]["chunkIds"],
        "duplicateSkipsProcessing": bool(changes["duplicate"]["result"].get("processing", {}).get("alreadyCompleted")),
        "restoredChunksMatchBaseline": chunk_ids(graph) == set(changes["restored"]["chunkIds"]),
    }
    changes["checks"] = checks
    report["update"] = changes
    report["collections"] = driver.call("collections")["result"]
    report["seconds"] = time.perf_counter() - started
    report["status"] = "completed"
    save_json(output / "report.json", report)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--resume", action="store_true", help="Continue evaluation of a completed, unchanged ingestion")
    args = parser.parse_args()
    output = args.output.resolve()
    if not args.resume and ((output / "runtime").exists() or (output / "report.json").exists()):
        parser.error("Use a fresh output directory; failed evidence must remain inspectable")
    (output / "operations").mkdir(parents=True, exist_ok=True)
    documents = json.loads((args.snapshot / "documents.json").read_text())
    save_json(output / "documents.json", documents)
    gateway = AuditedGateway("http://127.0.0.1:11234/v1", output / "model-calls")
    endpoint = gateway.start()
    driver = Driver(args.binary.resolve(), output, endpoint)
    report = {"framework": "cognee-rs", "commit": "c3165875027bfff353f407d031b60fecad0c8b96",
              "documents": documents, "queryWorkspace": str(Path(__file__).resolve().parents[3]),
              "runtime": {"llm": MLX_LLM, "embedding": MLX_EMBEDDER, "dimensions": 1024,
                          "modelRuntimeIncluded": False}}
    if args.resume:
        previous = json.loads((output / "report.json").read_text())
        if previous.get("documents") != documents or not previous.get("ingestion"):
            raise ValueError("Resume requires the same completed ingestion and exact document snapshot")
        if previous.get("update"):
            raise ValueError("Resume after a started update requires a separate recovery procedure")
        report["ingestion"] = previous["ingestion"]
        report["previousEvaluationError"] = previous.get("error")
        save_json(output / "evaluation-attempt-1.json", previous)
    try:
        evaluate(driver, output, documents, report)
        driver.close()
        driver = Driver(args.binary.resolve(), output, endpoint)
        restarted_rows, restarted_mapping = rows_by_run(driver)
        restarted = snapshot(driver, output, "graph-restarted")
        restored = json.loads((output / "update-graph-restored.json").read_text())
        report["restart"] = {
            "documentIdsStable": {r["id"] for r in restarted_rows["rows"]} ==
                {r["id"] for r in report["documentRows"]["rows"]},
            "nodeIdsStable": {n["id"] for n in restarted["nodes"]} == {n["id"] for n in restored["nodes"]},
            "chunkVectorsPresent": len(driver.call("vectors", ids=sorted(chunk_ids(restarted)))["result"]) == len(chunk_ids(restarted)),
        }
        # Match Python's supplemental hybrid check: both run after restoration.
        restarted_owners = ownership_map(restarted, restarted_mapping)
        report["initialNativeHybridMetrics"] = report["metrics"]["nativeHybrid"]
        for case in report["queries"]:
            response = driver.call("search", query=case["query"], type="HYBRID_COMPLETION")
            raw = response["result"]
            ranked = [{"objects_result": items, "evidence": []}
                      for items in (raw.get("context") or {}).values()]
            ranked.append({"objects_result": [], "evidence":raw.get("evidence", [])})
            case["routes"]["nativeHybridInitial"] = case["routes"]["nativeHybrid"]
            case["routes"]["nativeHybrid"] = {
                "runIds":candidate_ids(ranked, documents, restarted_owners),
                "raw":raw, "seconds":response["seconds"],
            }
        report["metrics"]["nativeHybrid"] = summarize_metrics(report["queries"], "nativeHybrid")
        report["nativeHybridStage"] = "afterRestoreAndRestart"
        save_json(output / "report.json", report)
    except Exception as error:
        import traceback
        traceback.print_exc()
        report.update(status="failed", error=str(error))
        save_json(output / "report.json", report)
        raise
    finally:
        driver.close()
        gateway.close()
        import resource
        report["driverPeakRssBytes"] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        save_json(output / "report.json", report)


if __name__ == "__main__":
    main()
