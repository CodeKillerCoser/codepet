"""Read-only TaskRun export. All framework IDs remain separate from business IDs."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


def digest(value: bytes | str) -> str:
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


def read_ledger(root: Path) -> tuple[list[dict], dict[str, str]]:
    """Snapshot complete v4 files; never instantiate the mutating Rust Store."""
    documents, source_hashes = [], {}
    for path in sorted(root.glob("projects/*/tasks/*.json")):
        raw = path.read_bytes()
        task = json.loads(raw)
        if task.get("schemaVersion") != 4 or not task.get("runs"):
            raise ValueError(f"Expected canonical v4 Task with runs: {path.name}")
        source_hashes[str(path)] = digest(raw)
        for index, run in enumerate(task["runs"]):
            doc = {
                "runId": run["id"],
                "taskId": task["id"],
                "projectId": task["projectId"],
                "taskTitle": task["title"],
                "taskDescription": task.get("description", ""),
                "title": run["title"],
                "kind": run["kind"],
                "goal": run.get("goal", ""),
                "solution": run.get("solution", ""),
                "description": run.get("description", ""),
                "status": run["status"],
                "workspace": run.get("agentWorkspace", {}).get("path", ""),
                "createdAt": run["createdAt"],
                "updatedAt": run["updatedAt"],
                "sourceRevision": task["revision"],
                "isCurrentRun": index == len(task["runs"]) - 1,
                "activityCount": len(run.get("timeline", [])),
                "activity": [
                    {k: event.get(k) for k in ("at", "kind", "summary", "reference")}
                    for event in run.get("timeline", [])
                ],
                "criteria": run.get("validationPlan", {}).get("items", []),
            }
            doc["text"] = render_document(doc)
            doc["contentHash"] = digest(doc["text"])
            documents.append(doc)
    if not documents:
        raise ValueError(f"No v4 TaskRuns found at {root}")
    return documents, source_hashes


def render_document(doc: dict) -> str:
    """Preserve original Chinese text; labels do not manufacture inferred facts."""
    sections = [
        ("Task topic / 所属主题", doc["taskTitle"]),
        ("Background / 背景", doc["taskDescription"]),
        ("Run / 本轮标题", doc["title"]),
        ("Goal / 目标", doc["goal"]),
        ("Solution / 方案", doc["solution"]),
        ("Description / 补充说明", doc["description"]),
    ]
    parts = [f"# CodePet TaskRun\nRun ID: {doc['runId']}"]
    parts.extend(f"## {label}\n{value}" for label, value in sections if value)
    # Keep the latest actual progress once. Historical activity remains in the
    # snapshot for comparison, rather than dominating text by repetition count.
    meaningful = [
        item for item in doc["activity"]
        if item.get("summary") and item.get("kind") not in {"workspace", "imported_workspace"}
        and not str(item.get("kind", "")).startswith("imported")
        and item["summary"] != "历史工作区观测"
    ]
    if meaningful:
        parts.append("## Latest recorded progress / 最近记录\n" + meaningful[-1]["summary"])
    if doc["criteria"]:
        criteria = [
            " | ".join(str(item.get(key, "")) for key in ("title", "steps", "expected"))
            for item in doc["criteria"]
        ]
        parts.append("## Validation criteria / 验收定义\n" + "\n".join(criteria))
    return "\n\n".join(parts)


def render_full_document(doc: dict) -> str:
    """Finish-time snapshot candidate, preserving meaningful ordered history."""
    sections = [
        ("所属主题", doc["taskTitle"]), ("背景", doc["taskDescription"]),
        ("本轮标题", doc["title"]), ("当前状态（不等于人工验收）", doc["status"]),
        ("目标（计划）", doc["goal"]), ("方案（计划，实施证据见进度）", doc["solution"]),
        ("补充说明", doc["description"]),
    ]
    parts = [f"# CodePet TaskRun\nRun ID: {doc['runId']}"]
    parts.extend(f"## {label}\n{value}" for label, value in sections if value)
    events, seen = [], set()
    for event in doc["activity"]:
        summary, kind = event.get("summary"), str(event.get("kind") or "")
        if not summary or summary == "历史工作区观测" or "workspace" in kind or kind.startswith("imported"):
            continue
        key = (event.get("at"), kind, summary)
        if key in seen:
            continue
        seen.add(key)
        events.append(f"- 时间 {event.get('at', '未知')} / {kind}：{summary}")
    if events:
        parts.append("## 有效进度历史（按源顺序，Agent 报告不等于人工验收）\n" + "\n".join(events))
    if doc["criteria"]:
        rows = [" | ".join(str(item.get(key, "")) for key in ("title", "steps", "expected"))
                for item in doc["criteria"]]
        parts.append("## 预期验收条件（不是通过证据）\n" + "\n".join(rows))
    return "\n\n".join(parts)


def build_queries(documents: list[dict]) -> list[dict]:
    """Explicit expected runs are annotated from source titles, before retrieval."""
    specifications = [
        ("http", "不再用 command args 启动服务，改成应用后台提供 URL 接入", ["任务 MCP 改为本机 HTTP"]),
        ("state", "切到别的页面再回来，之前扫描出的代码状态和筛选条件还在", ["保留 Codex 工作台代码状态扫描结果"]),
        ("instructions", "用户修改给 coding agent 的使用提示词，重新连接后生效", ["调整任务执行详情并支持编辑 Agent 接入提示词"]),
        ("drawer", "抽屉默认占半个窗口，拖宽之后双击恢复默认，预览文件别被挡住", ["统一全局侧栏卡片、动画与弹窗层级"]),
        ("protocol", "任务是长期目标，每轮处理有独立工作区、验证证据和执行记录", ["实现 Task / TaskRun 与四工具 Agent MCP", "调整任务执行详情并支持编辑 Agent 接入提示词"]),
        ("acceptance", "开发完成进入待验收，对话关联必填，验证状态和结论一起保存", ["收紧任务 MCP 字段并增加执行完成工具"]),
        ("navigation", "筛选放进工具栏，二级导航由应用内容框架统一提供", ["收紧任务页布局并统一内容区二级导航"]),
        ("unrelated", "新增银行卡自动扣款、发票导出和财务对账", []),
    ]
    queries = []
    for key, query, expected_titles in specifications:
        expected = [doc["runId"] for doc in documents if doc["taskTitle"] in expected_titles]
        if expected_titles and not expected:
            continue
        queries.append({"id": key, "query": query, "expectedRunIds": expected})
    return queries


def source_is_unchanged(source_hashes: dict[str, str]) -> bool:
    return all(Path(path).exists() and digest(Path(path).read_bytes()) == value
               for path, value in source_hashes.items())
