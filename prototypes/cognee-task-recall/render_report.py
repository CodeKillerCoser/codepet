"""Render inspectable probe evidence without external JS or network requests."""

import argparse
import json
from pathlib import Path



def knowledge_documents(documents: list[dict], extractions: list[dict]) -> list[dict]:
    """Aggregate all original chunks, excluding synthetic updates and duplicates."""
    result = []
    for doc in documents:
        original = " ".join(doc["text"].split())
        chunks, seen = [], set()
        for extracted in extractions:
            source = " ".join(extracted["source"].split())
            if source and source in original and source not in seen:
                seen.add(source)
                chunks.append(extracted)
        if not chunks:
            continue
        lines = [f"# {doc['taskTitle']} · LLM 加工结果", f"Run ID: {doc['runId']}",
                 "这是模型抽取结果，需要核对证据，不代表人工验收。"]
        for index, extracted in enumerate(chunks, 1):
            names = {node["id"]: node.get("name", node["id"]) for node in extracted["nodes"]}
            lines += [f"## 文本块 {index} / 回执 {extracted['call']}", "### 实体"]
            lines += [f"- {node.get('name')}（{node.get('type')}）：{node.get('description')}"
                      for node in extracted["nodes"]]
            lines.append("### 关系、状态与原文证据")
            for edge in extracted["edges"]:
                lines.append(f"- {names.get(edge['source_node_id'], '?')} → {edge['relationship_name']} → "
                             f"{names.get(edge['target_node_id'], '?')}\n  {edge.get('description', '')}")
        result.append({"runId": doc["runId"], "text": "\n\n".join(lines),
                       "calls": [chunk["call"] for chunk in chunks]})
    return result


def render(output: Path) -> Path:
    report = json.loads((output / "report.json").read_text())
    graph_path = output / "graph-baseline.json"
    graph = json.loads(graph_path.read_text()) if graph_path.exists() else {}
    assessment = {"unsupportedControlRelations": []}
    recheck = output / "update-fixture-recheck/report.json"
    if recheck.exists():
        assessment["updateRecheck"] = json.loads(recheck.read_text())
    llm_path = output / "llm-assessment.json"
    if llm_path.exists():
        assessment["llm"] = json.loads(llm_path.read_text())
        assessment["knowledgeDocuments"] = knowledge_documents(report["documents"], assessment["llm"]["extractions"])
        directory = output / "knowledge-documents"
        directory.mkdir(exist_ok=True)
        for doc in assessment["knowledgeDocuments"]:
            (directory / f"{doc['runId']}.md").write_text(doc["text"], encoding="utf-8")
    for stage in ("before", "after"):
        source = output / f"update-graph-{stage}.json"
        if not source.exists():
            continue
        snapshot = json.loads(source.read_text())
        names = {str(node["id"]): node.get("name", "") for node in snapshot.get("nodes", [])}
        for edge in snapshot.get("links", []):
            target_name = names.get(str(edge["target"]), "")
            if (edge.get("edge_class") == "semantic" and "probe" in target_name.lower()
                    and edge.get("relation") != "uses"):
                assessment["unsupportedControlRelations"].append({
                    "stage": stage, "source": names.get(str(edge["source"])),
                    "relation": edge.get("relation"), "target": target_name,
                    "expected": "The control text only states CodePet uses each library.",
                })
    (output / "assessment.json").write_text(json.dumps(assessment, ensure_ascii=False, indent=2))
    data = json.dumps({"report": report, "graph": graph, "assessment": assessment}, ensure_ascii=False).replace("<", "\\u003c")
    template = Path(__file__).with_name("report.html").read_text()
    target = output / "index.html"
    target.write_text(template.replace("__PROBE_DATA__", data), encoding="utf-8")
    return target


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    print(render(parser.parse_args().output.resolve()))
