"""Produce a source-backed comparison and links to both native visualizations."""
from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from mlx_gateway import assess_calls
from engine import save_json


def summarize(root, framework):
    report = json.loads((root / "report.json").read_text())
    graph = json.loads((root / "graph-baseline.json").read_text())
    calls = assess_calls(root / "model-calls")
    save_json(root / "llm-assessment.json", calls)
    names = {n["id"]: n.get("name", n["id"]) for n in graph["nodes"]}
    entities = [n for n in graph["nodes"] if n.get("type") == "Entity"]
    edges = [e for e in graph["links"] if e.get("edge_class") == "semantic"]
    sources = [d["text"] for d in report["documents"]]
    import re
    missing, states = [], Counter()
    for edge in edges:
        text = edge.get("edge_info", {}).get("edge_text", "") or edge.get("description", "")
        quotes = re.findall(r"「([^」]+)」", text)
        state = re.search(r"状态\s*=\s*([^；;\n]+)", text)
        states[state.group(1).strip() if state else "missing"] += 1
        if not quotes or not any(all(q in source for q in quotes) for source in sources):
            missing.append(edge)
    return {
        "framework": framework, "status": report.get("status"), "graph": report.get("graph"),
        "metrics": report.get("metrics"), "update": report.get("update"),
        "restart": report.get("restart"), "checks": report.get("checks"),
        "ingestion": report.get("ingestion"), "seconds": report.get("seconds"),
        "calls": {k: v for k, v in calls.items() if k != "extractions"},
        "entities": [{"id": n["id"], "name": n.get("name"), "type": n.get("entity_type") or n.get("_semantic_type"),
                      "description": n.get("description", "")} for n in entities],
        "relations": [{"source": names.get(e["source"]), "target": names.get(e["target"]),
                       "relation": e.get("relation"), "evidence": e.get("edge_info", {}).get("edge_text", ""),
                       "id": e.get("edge_info", {}).get("edge_object_id")} for e in edges],
        "relationLiteralEvidenceFailures": len(missing),
        "relationStates": dict(states),
        "queries": [{"id": q["id"], "query": q["query"], "expectedRunIds": q["expectedRunIds"],
                     "routes": {k: {a:b for a,b in v.items() if a != "raw"} for k,v in q["routes"].items()},
                     "fused":q["fused"]} for q in report.get("queries", [])],
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("python", type=Path)
    parser.add_argument("rust", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    python, rust = summarize(args.python, "Python Cognee 1.6.10"), summarize(args.rust, "cognee-rs")
    import os
    artifact = lambda path: os.path.relpath(path.resolve(), output)
    package = output / "package-measurement.json"
    data = {"python":python, "rust":rust,
            "documents":json.loads((args.rust / "documents.json").read_text()),
            "package":json.loads(package.read_text()) if package.exists() else None,
            "paths":{"python":artifact(args.python / "cognee-graph.html"),
                     "rust":artifact(args.rust / "update-graph-restored.html"),
                     "pythonReport":artifact(args.python / "report.json"),
                     "rustReport":artifact(args.rust / "report.json")},
            "limitations":["9 个固定历史 Run，7 个正例和 1 个负例；不是生产准确率评估。",
                            "相同业务原文、模型、抽取提示词；框架分块与结构化输出封装可能不同。",
                            "节点或关系数量不等于质量；逐字引用存在不证明语义蕴含或人工验收。",
                            "自定义融合与框架原生 hybrid 分列，更新只影响实验副本。",
                            "抽取统计取初次建库；原生图均在受控更新、恢复原文后导出，重抽取可能改变节点数量。",
                            "Rust Semantic 投影视图尚未实现；包体积仅测当前 macOS arm64 构建。"]}
    save_json(output / "comparison.json", data)
    template = Path(__file__).with_name("report.html").read_text()
    embedded = json.dumps(data, ensure_ascii=False).replace("<", "\\u003c")
    (output / "index.html").write_text(template.replace("__COMPARISON__", embedded))
    print(output / "index.html")


if __name__ == "__main__":
    main()
