"""Additional native-hybrid and restart checks on the new Python control corpus."""
import argparse
import asyncio
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from engine import (MLX_EMBEDDER, candidate_ids, configure, document_rows, graph_snapshot,
                    ownership_map, save_json, stored_chunk_ids)
from mlx_gateway import AuditedGateway
from probe import summarize_metrics


async def evaluate(root):
    import cognee
    from uuid import UUID
    report = json.loads((root / "report.json").read_text())
    dataset = report["ingestion"]["datasetId"]
    rows = await document_rows(dataset)
    mapping = {r["dataId"]: r["externalMetadata"]["runId"] for r in rows}
    graph = await graph_snapshot(dataset)
    save_json(root / "graph-restarted.json", graph)
    owners = ownership_map(graph, mapping)
    for case in report["queries"]:
        response = await cognee.search(query_text=case["query"], query_type=cognee.SearchType.HYBRID_COMPLETION,
            dataset_ids=[UUID(dataset)], top_k=8, only_context=True, include_references=True, verbose=True)
        from engine import plain
        raw = plain(response)
        case["routes"]["nativeHybrid"] = {"runIds":candidate_ids(raw, report["documents"], owners),"raw":raw}
    report["metrics"]["nativeHybrid"] = summarize_metrics(report["queries"], "nativeHybrid")
    report["nativeHybridStage"] = "afterRestoreAndRestart"
    baseline = json.loads((root / "graph-baseline.json").read_text())
    chunks = {n["id"] for n in graph["nodes"] if n.get("type") == "DocumentChunk"}
    report["restart"] = {
        "documentIdsStable": {r["dataId"] for r in rows} == {r["dataId"] for r in report["documentRows"]},
        "restoredChunkIdsMatchBaseline":chunks == {n["id"] for n in baseline["nodes"] if n.get("type") == "DocumentChunk"},
        "chunkVectorsPresent": await stored_chunk_ids(list(chunks)) == chunks,
    }
    save_json(root / "report.json", report)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    gateway = AuditedGateway("http://127.0.0.1:11234/v1", root / "hybrid-model-calls")
    configure(root / "runtime", root / "cache-extra", MLX_EMBEDDER, backend="mlx", endpoint=gateway.start())
    try:
        asyncio.run(evaluate(root))
    finally:
        gateway.close()


if __name__ == "__main__":
    main()
