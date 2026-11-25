"""Run: python probe.py --source-root /absolute/data/tasks/v4.

The production ledger is only read. All models, databases and evidence are
written beneath --output-root. Synthetic changes affect only the copied corpus.
"""

from __future__ import annotations

import argparse
import asyncio
from collections import Counter
from datetime import datetime, timezone
from importlib.metadata import version
import json
from pathlib import Path
import sys
import time

from dataset import build_queries, digest, read_ledger, render_full_document, source_is_unchanged
from engine import (
    DEFAULT_EMBEDDER, MLX_EMBEDDER, MLX_LLM, configure, document_rows, fuse, graph_html, graph_snapshot,
    ingest, local_file, ownership_map, progress, save_json, search_routes, stored_chunk_ids, update_document,
)
from readonly import ReadOnlyLedger


def summarize_metrics(cases: list[dict], lane: str) -> dict:
    positives = [case for case in cases if case["expectedRunIds"]]
    ranks = []
    for case in positives:
        ids = ([item["runId"] for item in case["fused"]] if lane == "fused"
               else case["routes"][lane]["runIds"])
        ranks.append(next((rank for rank, run_id in enumerate(ids, 1)
                           if run_id in case["expectedRunIds"]), None))
    count = len(ranks)
    return {"cases": count, "hitAt1": sum(rank == 1 for rank in ranks) / count if count else None,
            "hitAt3": sum(rank is not None and rank <= 3 for rank in ranks) / count if count else None,
            "mrr": sum(1 / rank for rank in ranks if rank) / count if count else None,
            "note": "Small source-annotated smoke set, not a general quality benchmark."}


def graph_counts(graph: dict) -> dict:
    return {"nodes": len(graph.get("nodes", [])), "edges": len(graph.get("links", [])),
            "types": dict(Counter(str(node.get("type", "unknown")) for node in graph.get("nodes", []))),
            "edgeClasses": dict(Counter(str(edge.get("edge_class", "unknown")) for edge in graph.get("links", []))),
            "relations": dict(Counter(str(edge.get("relation", "unknown")) for edge in graph.get("links", []))),
            "entities": [{"id": node["id"], "name": node.get("name")} for node in graph.get("nodes", [])
                         if node.get("type") == "Entity"]}


def make_update_fixture(original: str) -> tuple[str, str]:
    """Change specific width statements without touching dates or other ratios."""
    prefix = "\n\n## Controlled update fixture / 仅用于隔离验证，不是生产事实\n"
    before = original + prefix + (
        "CodePet uses ProbeOldLibrary. ProbeOldLibrary is a software library. "
        "CodePet uses ProbeSharedLibrary. ProbeSharedLibrary is a software library.\n"
        "ProbeDocumentVersion: CODEPET_PROBE_OLD_REVISION"
    )
    revised = original
    for old, new in (
        ("抽屉默认宽度改为窗口宽度的 1/2", "抽屉默认宽度改为窗口宽度的 3/5"),
        ("抽屉默认宽度及双击复位已按用户要求改为窗口的 1/2", "抽屉默认宽度及双击复位已按用户要求改为窗口的 3/5"),
        ("首次挂载 CSS 后备 50vw", "首次挂载 CSS 后备 60vw"),
    ):
        revised = revised.replace(old, new)
    after = revised + prefix + (
        "CodePet uses ProbeNewLibrary. ProbeNewLibrary is a software library. "
        "CodePet uses ProbeSharedLibrary. ProbeSharedLibrary is a software library.\n"
        "ProbeDocumentVersion: CODEPET_PROBE_NEW_REVISION"
    )
    return before, after


async def update_experiment(target: dict, data_id: str, dataset_id: str, output: Path) -> dict:
    """Controlled replacement, no-op, and restoration on one copied real run."""
    original = target["text"]
    before, after = make_update_fixture(original)
    (output / "update-input-before.md").write_text(before, encoding="utf-8")
    (output / "update-input-after.md").write_text(after, encoding="utf-8")
    result = {"runId": target["runId"], "dataId": data_id,
              "synthetic": True, "note": "Only this copied document is mutated; production facts are unchanged."}
    try:
        progress("update_fixture_before", runId=target["runId"])
        result["baseline"] = await update_document(data_id, dataset_id, before)
        graph_before = await graph_snapshot(dataset_id)
        save_json(output / "update-graph-before.json", graph_before)
        progress("update_fixture_after")
        result["changed"] = await update_document(data_id, dataset_id, after)
        graph_after = await graph_snapshot(dataset_id)
        save_json(output / "update-graph-after.json", graph_after)
        rows = await document_rows(dataset_id)
        row = next(row for row in rows if row["dataId"] == data_id)
        stored = local_file(row["rawDataLocation"]).read_text(encoding="utf-8")
        result["checks"] = {
            "sameDataId": row["dataId"] == data_id,
            "latestTextStored": stored == after,
            "oldMarkerRemovedFromStoredText": "CODEPET_PROBE_OLD_REVISION" not in stored,
            "newMarkerStored": "CODEPET_PROBE_NEW_REVISION" in stored,
        }
        graph_text = "\n".join(str(node.get(key, "")) for node in graph_after.get("nodes", [])
                               for key in ("text", "name", "description", "summary"))
        result["checks"]["oldMarkerRemovedFromGraphContent"] = "CODEPET_PROBE_OLD_REVISION" not in graph_text
        before_entities = {node.get("name", "").lower(): str(node["id"])
                           for node in graph_before.get("nodes", []) if node.get("type") == "Entity"}
        after_entities = {node.get("name", "").lower(): str(node["id"])
                          for node in graph_after.get("nodes", []) if node.get("type") == "Entity"}
        result["entityChecks"] = {
            "oldEntityExtracted": "probeoldlibrary" in before_entities,
            "oldEntityRemoved": "probeoldlibrary" not in after_entities,
            "newEntityExtracted": "probenewlibrary" in after_entities,
            "sharedEntityIdRetained": before_entities.get("probesharedlibrary") is not None
            and before_entities.get("probesharedlibrary") == after_entities.get("probesharedlibrary"),
        }
        before_nodes = {str(node["id"]) for node in graph_before.get("nodes", [])}
        after_nodes = {str(node["id"]) for node in graph_after.get("nodes", [])}
        before_chunks = {str(node["id"]) for node in graph_before.get("nodes", []) if node.get("type") == "DocumentChunk"}
        after_chunks = {str(node["id"]) for node in graph_after.get("nodes", []) if node.get("type") == "DocumentChunk"}
        removed_chunks, retained_chunks = before_chunks - after_chunks, before_chunks & after_chunks
        result["checks"]["removedChunkVectorsDeleted"] = not await stored_chunk_ids(list(removed_chunks))
        result["checks"]["retainedChunkVectorsPresent"] = await stored_chunk_ids(list(retained_chunks)) == retained_chunks
        result["graphDelta"] = {"before": graph_counts(graph_before), "after": graph_counts(graph_after),
                                "removedNodes": len(before_nodes - after_nodes),
                                "addedNodes": len(after_nodes - before_nodes),
                                "retainedNodes": len(before_nodes & after_nodes)}
        result["noop"] = await update_document(data_id, dataset_id, after)
        result["checks"]["noopReportedUnchanged"] = result["noop"]["result"].get("status") == "unchanged"
        graph_noop = await graph_snapshot(dataset_id)
        result["checks"]["noopNodeIdsUnchanged"] = after_nodes == {
            str(node["id"]) for node in graph_noop.get("nodes", [])}
        result["queries"] = await search_routes("CODEPET_PROBE_NEW_REVISION ProbeNewLibrary", dataset_id, [target], {data_id: target["runId"]})
    finally:
        progress("restore_copied_document")
        result["restore"] = await update_document(data_id, dataset_id, original)
        restored = next(row for row in await document_rows(dataset_id) if row["dataId"] == data_id)
        result.setdefault("checks", {})["originalCopyRestored"] = local_file(restored["rawDataLocation"]).read_text() == original
    return result


async def evaluate(args, output: Path, report: dict) -> None:
    documents = report["documents"]
    if not report.get("ingestion"):
        progress("ingestion_started", documents=len(documents))
        report["ingestion"] = await ingest(documents, "codepet_task_runs")
    else:
        progress("resume_existing_corpus", documents=len(documents))
    dataset_id = report["ingestion"]["datasetId"]
    save_json(output / "report.json", report)
    rows = await document_rows(dataset_id)
    report["documentRows"] = rows
    data_ids = {row["dataId"]: row["externalMetadata"]["runId"] for row in rows
                if isinstance(row["externalMetadata"], dict) and row["externalMetadata"].get("runId")}
    report["checks"]["allRunsMappedToFrameworkDocuments"] = set(data_ids.values()) == {
        doc["runId"] for doc in documents}
    graph = await graph_snapshot(dataset_id)
    save_json(output / "graph-baseline.json", graph)
    report["graph"] = graph_counts(graph)
    source_ids = ownership_map(graph, data_ids)
    save_json(output / "source-ownership.json", source_ids)
    progress("graph_extracted", **report["graph"])
    report["queries"] = []
    def chat_count():
        return sum(json.loads(path.read_text()).get("route") == "chat/completions"
                   for path in (output / "model-calls").glob("*.json"))
    calls_before_recall = chat_count()
    for case in build_queries(documents):
        progress("query", id=case["id"])
        routes = await search_routes(case["query"], dataset_id, documents, source_ids)
        fused = fuse(routes, documents, args.workspace)
        # Reuse the same semantic evidence to test a competing *existing*
        # workspace prior. No extra model calls and no hard workspace scope.
        common_workspace = Counter(doc["workspace"] for doc in documents).most_common(1)[0][0]
        case.update(routes=routes, fused=fused,
                    competingWorkspace=common_workspace,
                    fusedWithCompetingWorkspace=fuse(routes, documents, common_workspace))
        report["queries"].append(case)
        save_json(output / "report.json", report)
    report["metrics"] = {lane: summarize_metrics(report["queries"], lane)
                         for lane in ("semantic", "lexical", "graph", "fused")}
    if args.backend == "mlx":
        report["checks"]["recallDoesNotCallLLM"] = chat_count() == calls_before_recall
    report["checks"]["allRecallRoutesSucceeded"] = all(
        not route.get("error") for case in report["queries"] for route in case["routes"].values()
    )
    if not args.skip_update:
        target = next((doc for doc in documents if "统一全局侧栏" in doc["taskTitle"]), documents[0])
        data_id = next(key for key, value in data_ids.items() if value == target["runId"])
        try:
            report["update"] = await update_experiment(target, data_id, dataset_id, output)
        except Exception as error:
            report["update"] = {"error": f"{type(error).__name__}: {error}"}
        report["checks"]["updateMechanicsPassed"] = bool(report["update"].get("checks")) and all(
            report["update"].get("checks", {}).values()
        )
        report["checks"]["controlledEntityReplacementPassed"] = bool(report["update"].get("entityChecks")) and all(
            report["update"].get("entityChecks", {}).values()
        )
    save_json(output / "report.json", report)
    progress("rendering_framework_graph")
    try:
        await graph_html(dataset_id, output / "cognee-graph.html")
        report["frameworkGraphHtml"] = "cognee-graph.html"
    except Exception as error:
        report["frameworkGraphHtmlError"] = f"{type(error).__name__}: {error}"
    report["checks"]["frameworkGraphExported"] = bool(report.get("frameworkGraphHtml"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, default=Path(__file__).resolve().parents[2] / "artifacts/cognee-task-recall-qa")
    parser.add_argument("--workspace", default=str(Path(__file__).resolve().parents[2]))
    parser.add_argument("--embedding-model", default=DEFAULT_EMBEDDER)
    parser.add_argument("--backend", choices=["gliner", "mlx"], default="gliner")
    parser.add_argument("--endpoint", default="http://127.0.0.1:11234/v1")
    parser.add_argument("--llm-model", default=MLX_LLM)
    parser.add_argument("--input-mode", choices=["latest", "history"], default="latest")
    parser.add_argument("--snapshot-from", type=Path, help="Reuse exported business records, with new isolated indexes")
    parser.add_argument("--reuse-model-calls-from", type=Path, help="Reuse exact matching successful local model calls from an earlier evidence directory")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--skip-update", action="store_true")
    parser.add_argument("--resume", type=Path, help="Reuse a previous isolated corpus and its original snapshot")
    args = parser.parse_args()
    if args.backend == "mlx" and args.embedding_model == DEFAULT_EMBEDDER:
        args.embedding_model = MLX_EMBEDDER
    if args.resume and args.snapshot_from:
        parser.error("Choose --resume or --snapshot-from, not both")
    source = args.source_root.expanduser().resolve()
    root = args.output_root.expanduser().resolve()
    if source == root or source in root.parents or root in source.parents:
        parser.error("Source ledger and artifact directory must be disjoint")
    guard = ReadOnlyLedger(source)
    guard.install()
    output = (args.resume.resolve() if args.resume else
              root / "runs" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ"))
    if root not in output.parents:
        parser.error("Resume must name a run directory under --output-root")
    output.mkdir(parents=True, exist_ok=bool(args.resume))
    snapshot = args.resume or args.snapshot_from
    if snapshot:
        documents = json.loads((snapshot / "documents.json").read_text())
        hashes = json.loads((snapshot / "source-hashes.json").read_text())
    else:
        documents, hashes = read_ledger(source)
    if not args.resume and args.input_mode == "history":
        for doc in documents:
            doc["text"] = render_full_document(doc)
            doc["contentHash"] = digest(doc["text"])
    save_json(output / "documents.json", documents)
    save_json(output / "source-hashes.json", hashes)
    for doc in documents:
        folder = output / "documents"
        folder.mkdir(exist_ok=True)
        (folder / f"{doc['runId']}.md").write_text(doc["text"], encoding="utf-8")
    bridge, endpoint = None, args.endpoint
    if args.backend == "mlx":
        from mlx_gateway import AuditedGateway
        port = 0
        if args.resume:
            from urllib.parse import urlparse
            previous_runtime = json.loads((output / "report.json").read_text())["runtime"]
            if previous_runtime.get("backend") != "mlx":
                parser.error("Resume must use the original backend")
            expected = {"extractor": args.llm_model, "embeddingModel": args.embedding_model,
                        "inputMode": args.input_mode,
                        "promptHash": digest(Path(__file__).with_name("development_graph_prompt.txt").read_text())}
            if any(previous_runtime.get(key) != value for key, value in expected.items()):
                parser.error("Resume must use the original models, input mode and prompt; use --snapshot-from for a comparison")
            port = urlparse(previous_runtime.get("auditEndpoint", "")).port or 0
        reuse = args.reuse_model_calls_from / "model-calls" if args.reuse_model_calls_from else None
        bridge = AuditedGateway(endpoint, output / "model-calls", port=port, reuse=reuse)
        endpoint = bridge.start()
    configure(output / "runtime", root / "models", args.embedding_model,
              backend=args.backend, endpoint=endpoint, llm_model=args.llm_model)
    report = {"createdAt": datetime.now(timezone.utc).isoformat(), "output": str(output),
              "sourceRoot": str(source), "queryWorkspace": args.workspace,
              "documents": documents, "checks": {}, "limitations": [
                  "Small source-annotated smoke test; not a production accuracy benchmark.",
                  "Model and embedding changes are bundled; use the same-model input comparison to isolate input effects.",
                  "Negative queries report candidate behavior; no relevance rejection threshold is calibrated.",
              ], "runtime": {"cognee": version("cognee"), "embeddingModel": args.embedding_model,
                             "embeddingProvider": "openai_compatible" if bridge else "fastembed",
                             "extractor": args.llm_model if bridge else "gliner_demo",
                             "backend": args.backend, "inputMode": args.input_mode,
                             "graphStore": "ladybug", "vectorStore": "lancedb"}}
    if bridge:
        prompt = Path(__file__).with_name("development_graph_prompt.txt").read_text()
        (output / "extraction-prompt.txt").write_text(prompt, encoding="utf-8")
        report["runtime"].update(endpoint=args.endpoint, promptHash=digest(prompt),
                                  embeddingDimensions=1024, thinking=False, auditEndpoint=endpoint)
    if args.snapshot_from:
        baseline = json.loads((args.snapshot_from / "report.json").read_text())
        report["comparison"] = {"baselineOutput": str(args.snapshot_from.resolve()),
                                "metrics": baseline.get("metrics"), "graph": baseline.get("graph"),
                                "runtime": baseline.get("runtime"), "previous": baseline.get("comparison"),
                                "inputMode": baseline["runtime"].get("inputMode", "latest"),
                                "note": "Same exported business snapshot; model, embedding, prompt and chunk size may differ."}
    if args.resume:
        previous = json.loads((output / "report.json").read_text())
        report["ingestion"] = previous.get("ingestion")
        report["previousAttempts"] = previous.get("previousAttempts", []) + [{
            "status": previous.get("status"), "seconds": previous.get("seconds"),
            "updateError": previous.get("update", {}).get("error"),
        }]
    started = time.perf_counter()
    exit_code = 0
    try:
        if not args.prepare_only:
            asyncio.run(evaluate(args, output, report))
        report["status"] = "prepared" if args.prepare_only else "completed"
        if report.get("update", {}).get("error") or any(value is False for value in report["checks"].values()):
            report["status"] = "completed_with_failures"
            exit_code = 1
    except Exception as error:
        import traceback
        report["status"] = "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        (output / "error.txt").write_text(traceback.format_exc(), encoding="utf-8")
        exit_code = 1
    except KeyboardInterrupt:
        report["status"] = "interrupted"
        exit_code = 130
    finally:
        if bridge:
            bridge.close()
            from mlx_gateway import assess_calls
            assessment = assess_calls(output / "model-calls")
            save_json(output / "llm-assessment.json", assessment)
            report["llmAssessment"] = {key: value for key, value in assessment.items() if key != "extractions"}
        report["seconds"] = time.perf_counter() - started
        report["sourceComparison"] = {
            "unchanged": source_is_unchanged(hashes),
            "changedPaths": [path for path, old_hash in hashes.items()
                             if not Path(path).exists() or digest(Path(path).read_bytes()) != old_hash],
            "note": "External writers may update the live ledger while this probe evaluates its fixed snapshot.",
        }
        report["checks"]["productionReadOnlyGuard"] = True
        report["checks"]["noProductionWriteAttempts"] = guard.blocked_writes == 0
        if report.get("status") == "completed" and any(value is False for value in report["checks"].values()):
            report["status"], exit_code = "completed_with_failures", 1
        save_json(output / "report.json", report)
        from render_report import render
        render(output)
        save_json(root / "latest.json", {"output": str(output), "status": report["status"]})
        progress("finished", status=report["status"], output=str(output), error=report.get("error"))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
