#!/usr/bin/env python3
"""Measure real local stdio MCP calls against an isolated ledger and large merge commits."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time


def git(repo, *args):
    return subprocess.check_output(["git", "-C", repo, *args], text=True).strip()


def fixture(repo, merge_sizes):
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.name", "MCP benchmark")
    git(repo, "config", "user.email", "benchmark@example.invalid")
    (repo / "owned.txt").write_text("base\n")
    git(repo, "add", ".")
    git(repo, "commit", "-qm", "base")
    commits = []
    for batch, size in enumerate(merge_sizes):
        branch = f"incoming-{batch}"
        git(repo, "checkout", "-qb", branch)
        incoming = repo / branch
        incoming.mkdir()
        for index in range(size):
            (incoming / f"file-{index:04d}.txt").write_text(f"merge content {index}\n")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "incoming changes")
        git(repo, "checkout", "-q", "main")
        git(repo, "merge", "-q", "--no-ff", branch, "-m", "large merge")
        commits.append(git(repo, "rev-parse", "HEAD"))
    (repo / "owned.txt").write_text("dirty\n")
    return commits


def benchmark(binary, samples, merge_sizes, verbose=False):
    with tempfile.TemporaryDirectory(prefix="codepet-mcp-latency-") as tmp:
        base = Path(tmp)
        repo = base / "repo"
        merges = fixture(repo, merge_sizes)
        merge = merges[-1]
        home = base / "home"
        home.mkdir()
        env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), CODEX_HOME=str(home / ".codex"),
                   CODEPET_AGENT="codex", XDG_DATA_HOME=str(home), LOCALAPPDATA=str(home))
        env.pop("CODEPET_TASK_RECALL_BUNDLE", None)
        timings = {}
        with subprocess.Popen(
            [binary, "--data-dir", base / "ledger"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, env=env,
        ) as process:
            number = 0

            def rpc(method, params, label=None):
                nonlocal number
                number += 1
                request = {"jsonrpc": "2.0", "id": number, "method": method, "params": params}
                start = time.perf_counter_ns()
                process.stdin.write(json.dumps(request).encode() + b"\n")
                process.stdin.flush()
                while True:
                    if not select.select([process.stdout], [], [], 120)[0]:
                        process.kill()
                        raise TimeoutError(label or method)
                    line = process.stdout.readline()
                    response = json.loads(line)
                    if response.get("id") == number:
                        break
                elapsed = (time.perf_counter_ns() - start) / 1_000_000
                if "error" in response:
                    raise RuntimeError(response["error"])
                if label and verbose:
                    print(f"{label:24} {elapsed:10.2f} ms   {len(line):6} B", file=sys.stderr)
                if label:
                    timings.setdefault(label, []).append((elapsed, len(line)))
                return response["result"]

            def call(name, args, label=None, expect_error=None):
                result = rpc("tools/call", {"name": name, "arguments": args}, label or name)
                message = result["content"][0]["text"]
                if expect_error:
                    assert result.get("isError") and expect_error in message, result
                    return None
                if result.get("isError"):
                    raise RuntimeError(result)
                return json.loads(message)

            rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                               "clientInfo": {"name": "codepet-benchmark", "version": "1"}})
            process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            process.stdin.flush()
            names = {tool["name"] for tool in rpc("tools/list", {})["tools"]}
            chat = ["00000000-0000-4000-8000-000000000001"]
            for index in range(samples):
                prefix = f"sample-{index}"
                create = dict(title=f"Latency {prefix}", agentWorkspace={"path": str(repo)},
                              initialRun={"agentConversation": chat, "validationPlan": {"items": [
                                  {"title": "Local check", "scope": "local",
                                   "steps": "Verify", "expected": "Pass"}]}})
                first = call("create-task", dict(create, operationId=prefix + "-create"))
                ids = {key: first[key] for key in ["taskId", "runId"]}
                context = dict(ids, agentConversation=chat, expectedRevision=first["revision"])
                normal = call("update-task-run", dict(context, note="Progress"), "update/notes")
                call("update-task-run", dict(context, expectedRevision=normal["revision"],
                     agentWorkspace={"path": str(repo), "relatedCommits": merges}), "update/merge")
                comment = call("comment-task-run", dict(ids, agentConversation=chat, comment="Ping"))
                result = call("find-task-runs", ids, "find/exact")
                assert result["runId"] == ids["runId"]
                call("find-task-runs", {"query": prefix}, "find/search")
                if "read-task-run-details" in names:
                    assert call("find-task-runs", {"runId": ids["runId"]}, "find/by-run")["runId"] == ids["runId"]
                    assert "agentWorkspace" not in result["run"] and "activity" not in result
                    for section in ["context", "activity", "git", "verification", "history"]:
                        call("read-task-run-details", dict(ids, section=section), f"details/{section}")
                    commit = call("read-task-run-details", dict(ids, section="git", commit=merge),
                                  "details/git-commit")
                    assert commit["total"] == merge_sizes[-1] and len(commit["files"]) == 20
                    artifact = base / "ledger/tasks/v4/diffs" / ids["taskId"] / ids["runId"]
                    version = json.loads((artifact / f"commit-{merge}.json").read_text())
                    assert len(version["files"]) == 300
                    assert "+merge content 0" in version["files"][0]["patch"]
                final_id = prefix + "-finish"
                finish = dict(context, expectedRevision=comment["revision"], conclusion="Local pass",
                              changedFiles=["owned.txt"], operationId=final_id, verification={
                                  "stage": "local", "target": merge, "environment": "macOS benchmark",
                                  "planRevision": first["planRevision"], "results": [
                                      {"criterionId": first["criterionIds"][0],
                                       "result": "passed", "evidence": "Local sample"}]})
                final = call("finish-task-run", finish)
                assert final["revision"] == comment["revision"] + 1
                if "get-task-operation" in names:
                    saved = call("get-task-operation", {"operationId": final_id})
                    assert saved["receipt"] == final
                    verified = call("find-task-runs", ids, "find/verified")
                    local = verified["run"]["validationPlan"]["latestByStage"]["local"]
                    assert local["results"][0]["result"] == "passed"
                    assert local["results"][0]["actor"] == "agent"
                    if index == 0:
                        lock = base / "ledger/tasks/v4/writer.lock"
                        with lock.open("rb") as writer:
                            fcntl.flock(writer, fcntl.LOCK_EX)
                            saved = call("get-task-operation", {"operationId": final_id},
                                         "get-operation/lock-held")
                            assert saved["receipt"] == final
                            call("find-task-runs", ids, "find/lock-held",
                                 expect_error="Task lineage is busy")
                publish = call("update-task-run", dict(context, expectedRevision=final["revision"],
                               status="published", delivery="fixture"), "update/publish")
                call("create-task-run", {"taskId": first["taskId"], "expectedRevision": publish["revision"],
                                         "title": "Follow-up", "agentConversation": chat})
            process.stdin.close()
            if process.wait(timeout=10):
                raise RuntimeError("MCP failed on shutdown")
    return {label: {"calls": len(rows), "maxMs": round(max(ms for ms, _ in rows), 2),
                    "maxReplyBytes": max(size for _, size in rows)}
            for label, rows in timings.items()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=lambda value: Path(value).resolve())
    parser.add_argument("--samples", type=int, default=8)
    parser.add_argument("--merge-sizes", type=int, nargs="+", default=[1200])
    parser.add_argument("--budget-ms", type=float)
    parser.add_argument("--verbose", action="store_true")
    options = parser.parse_args()
    if options.samples < 1:
        parser.error("--samples must be positive")
    if any(size < 300 for size in options.merge_sizes):
        parser.error("--merge-sizes expects at least 300 files per commit")
    report = benchmark(options.binary, options.samples, options.merge_sizes, options.verbose)
    print(json.dumps(report, indent=2))
    if options.budget_ms is not None and any(
        row["maxMs"] > options.budget_ms for row in report.values()
    ):
        raise SystemExit(1)
