"""Export existing experimental evidence once; native runtimes never import Python."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

import lancedb

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from dataset import build_queries


def prepare(source: Path, output: Path) -> None:
    report = json.loads((source / "report.json").read_text())
    graph = json.loads((source / "graph-baseline.json").read_text())
    owners = json.loads((source / "source-ownership.json").read_text())
    db = lancedb.connect(source / "runtime/system/databases/cognee.lancedb")
    chunks = {row["id"]: row for row in db.open_table("DocumentChunk_text").to_arrow().to_pylist()}
    entities = {row["id"]: row for row in db.open_table("Entity_name").to_arrow().to_pylist()}
    payload = {"documents": report["documents"], "queries": build_queries(report["documents"]),
               "chunks": [], "entities": [], "edges": [], "mentions": [],
               "workspace": report["queryWorkspace"], "source": str(source.resolve()),
               "baselineMetrics": report["metrics"], "baselineQueries": report["queries"]}
    for node in graph["nodes"]:
        if node["type"] == "DocumentChunk":
            row = chunks[node["id"]]
            assert row["payload"]["text"] == node["text"], "restored chunk differs from original snapshot"
            run_ids = owners[node["id"]]
            assert len(run_ids) == 1
            payload["chunks"].append({"id": node["id"], "runId": run_ids[0],
                                      "text": node["text"], "vector": row["vector"]})
        elif node["type"] == "Entity":
            payload["entities"].append({"id": node["id"], "name": node["name"],
                "type": node.get("entity_type", ""), "description": node.get("description", ""),
                "vector": entities.get(node["id"], {}).get("vector")})
    entity_ids = {e["id"] for e in payload["entities"]}
    chunk_ids = {c["id"] for c in payload["chunks"]}
    for edge in graph["links"]:
        if edge["source"] in chunk_ids and edge["target"] in entity_ids:
            payload["mentions"].append([edge["source"], edge["target"]])
        if edge.get("edge_class") == "semantic":
            evidence = edge.get("edge_info", {}).get("edge_text", "")
            quotes = re.findall(r"「([^」]+)」", evidence)
            # Provenance is evidence-text occurrence, never arbitrary graph ownership propagation.
            run_ids = [d["runId"] for d in report["documents"]
                       if quotes and all(q in d["text"] for q in quotes)]
            payload["edges"].append({"id": edge["edge_info"]["edge_object_id"],
                "source": edge["source"], "target": edge["target"], "relation": edge["relation"],
                "evidence": evidence, "runIds": run_ids,
                "provenanceMethod": "literal-quote occurrence; not entailment validation"})
    payload["fixtureHash"] = hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()
    output.mkdir(parents=True, exist_ok=True)
    (output / "fixture.json").write_text(json.dumps(payload, ensure_ascii=False))
    print(json.dumps({"chunks": len(payload["chunks"]), "entities": len(payload["entities"]),
                      "edges": len(payload["edges"]), "mentions": len(payload["mentions"]),
                      "fixtureHash": payload["fixtureHash"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    prepare(args.source, args.output)
