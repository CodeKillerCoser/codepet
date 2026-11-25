"""Snapshot native Cognee results by structural provenance, never by title or text IDs."""
import json


def metadata(node):
    value = node.get("external_metadata") or {}
    return json.loads(value) if isinstance(value, str) else value


def processing_result(graph, document):
    nodes = {str(node["id"]):node for node in graph.get("nodes", [])}
    owned = {key for key,node in nodes.items() if node.get("type") in ("TextDocument", "DocumentChunk")
             and metadata(node).get("runId") == document["runId"]
             and metadata(node).get("contentHash") == document["contentHash"]}
    chunks = {key for key in owned if nodes[key].get("type") == "DocumentChunk"}
    entities, summaries = set(), set()
    for edge in graph.get("links", []):
        source, target = str(edge["source"]), str(edge["target"])
        if source not in chunks:
            continue
        kind = nodes.get(target, {}).get("type")
        if kind == "Entity":
            entities.add(target)
        elif kind == "TextSummary":
            summaries.add(target)
    for key,node in nodes.items():
        if node.get("type") == "TextSummary" and str(node.get("source_chunk_id")) in chunks:
            summaries.add(key)
    summary_texts = list(dict.fromkeys(str(nodes[key].get("text") or nodes[key].get("name") or "")[:12000]
                                     for key in sorted(summaries)))
    entity_rows = [{"id":key, "name":str(nodes[key].get("name", ""))[:500],
                    "type":str(nodes[key].get("entity_type", ""))[:200],
                    "description":str(nodes[key].get("description") or "")[:2000]} for key in sorted(entities)]
    return {"source":"cognee", "description":f"文档已加工 · {len(entities)} 个关联实体 · 索引已更新",
            "summaries":summary_texts[:20], "entities":entity_rows[:200],
            "entityCount":len(entities), "chunkCount":len(chunks),
            "truncated":len(entity_rows) > 200 or len(summary_texts) > 20,
            "indexUpdated":True,
            "note":"摘要与实体来自本次文档版本的原生图；同名实体可能由多个执行共享。"}
