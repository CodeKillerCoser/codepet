"""Inspect native BM25 scores and normalize control-name comparisons after restoration."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run import Driver, node_names
from engine import candidate_ids, fuse, ownership_map, save_json
from mlx_gateway import AuditedGateway
from probe import summarize_metrics


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    report = json.loads((root / "report.json").read_text())
    if report.get("status") != "completed":
        raise ValueError("Finish the original evaluation before the score recheck")
    save_json(root / "report-before-score-recheck.json", report)
    baseline = json.loads((root / "graph-baseline.json").read_text())
    graph = json.loads((root / "graph-restarted.json").read_text())
    chunks = lambda g: sorted(n["text"] for n in g["nodes"] if n.get("type") == "DocumentChunk")
    if chunks(baseline) != chunks(graph):
        raise ValueError("Restored chunks must exactly match initial chunks for the lexical recheck")
    mapping = {r["id"]: json.loads(r["external_metadata"])["runId"]
               for r in report["documentRows"]["rows"]}
    owners = ownership_map(graph, mapping)
    gateway = AuditedGateway("http://127.0.0.1:11234/v1", root / "lexical-model-calls")
    driver = Driver(args.binary.resolve(), root, gateway.start())
    try:
        for query in report["queries"]:
            result = driver.call("search", query=query["query"], type="CHUNKS_LEXICAL")
            raw = result["result"]
            items = raw["result"]["data"]
            if any(x.get("score") is None for x in items):
                raise ValueError("Native lexical scores must be visible before zero-score filtering")
            positive = [x for x in items if x["score"] > 0]
            query["lexicalBeforeScoreVisibilityFix"] = query["routes"]["lexical"]
            query["routes"]["lexical"] = {
                "runIds": candidate_ids([{"objects_result":positive,"evidence":[]}], report["documents"], owners),
                "raw":raw,"seconds":result["seconds"], "discardedZeroScoreChunks":len(items)-len(positive),
            }
            query["fused"] = fuse({k:query["routes"][k] for k in ("semantic","lexical","graph")},
                                  report["documents"], report["queryWorkspace"])
        for lane in ("lexical", "fused"):
            report["metrics"][lane] = summarize_metrics(report["queries"], lane)
        for label in ("before", "after", "duplicate", "restored"):
            g = json.loads((root / f"update-graph-{label}.json").read_text())
            report["update"][label]["names"] = node_names(g)
        before = report["update"]["before"]["names"]
        after = report["update"]["after"]["names"]
        report["update"]["checks"].update({
            "oldEntityExtracted":"probeoldlibrary" in before,
            "oldEntityRemoved":"probeoldlibrary" not in after,
            "newEntityPresent":"probenewlibrary" in after,
            "sharedEntityControlExtracted":"probesharedlibrary" in before and "probesharedlibrary" in after,
            "sharedEntityIdStable":(before["probesharedlibrary"] == after["probesharedlibrary"])
                if "probesharedlibrary" in before and "probesharedlibrary" in after else None,
        })
        report["assessmentCorrections"] = [
            "Requested native BM25 with_scores=true; prior null scores were not zero relevance.",
            "Casefolded entity names, as the framework normalizes names to lowercase.",
            "A missing shared control entity makes ID retention untested, not a proven deletion defect.",
        ]
        report["lexicalRecheckRestoredChunksIdentical"] = True
        save_json(root / "report.json", report)
    finally:
        driver.close()
        gateway.close()


if __name__ == "__main__":
    main()
