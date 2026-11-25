// Opt-in real model integration: isolated ledger -> finish -> Rust-owned Cognee -> MCP recall.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { request as httpRequest } from "node:http";

const root = resolve(import.meta.dirname, "..");
const temporary = await mkdtemp(join(tmpdir(), "codepet-recall-mcp-"));
const workspace = join(temporary, "repo"), data = join(temporary, "data"), codex = join(temporary, "codex");
await Promise.all([workspace, data, codex].map(path => mkdir(path, { recursive: true })));
const git = (...args) => execFileSync("git", ["-C", workspace, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
git("init", "-b", "main"); git("config", "user.name", "Recall test"); git("config", "user.email", "test@example.invalid");
await writeFile(join(workspace, "readme"), "isolated recall fixture"); git("add", "."); git("commit", "-m", "fixture");
const conversation = randomUUID();
execFileSync("python3", ["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE threads(id TEXT,cwd TEXT)'); c.execute('INSERT INTO threads VALUES (?,?)',sys.argv[2:]); c.commit()", join(codex, "state_5.sqlite"), conversation, workspace]);
await mkdir(join(data, "model-gateway"));
const endpoint = "http://127.0.0.1:11234/v1";
await writeFile(join(data, "model-gateway/config.json"), JSON.stringify({ enabled: true, plainJson: true,
  llm: { endpoint, model: "ddalcu/Qwen3.8-27B-MLX-Serve-4bit" },
  embedding: { endpoint, model: "Qwen3-Embedding-0.6B-8bit", dimensions: 1024 },
  llmExtraBody: { chat_template_kwargs: { enable_thinking: false } } }), { mode: 0o600 });
const child = spawn(process.env.CODEPET_MCP_BINARY ?? join(root, "crates/target/debug/codepet-task-mcp"), ["--data-dir", data], {
  env: { ...process.env, CODEX_HOME: codex, CODEPET_AGENT: "codex",
    CODEPET_TASK_RECALL_BUNDLE: process.env.CODEPET_TASK_RECALL_BUNDLE ?? join(root, "artifacts/task-recall-bundle/native") },
  stdio: ["pipe", "pipe", "pipe"],
});
let stderr = "", next = 0;
const waiting = new Map();
child.stderr.on("data", chunk => { stderr = (stderr + chunk).slice(-4000); });
createInterface({ input: child.stdout }).on("line", line => {
  const message = JSON.parse(line), entry = waiting.get(message.id);
  if (!entry) return;
  clearTimeout(entry.timer); waiting.delete(message.id);
  message.error ? entry.reject(new Error(JSON.stringify(message.error))) : entry.resolve(message.result);
});
const send = message => child.stdin.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
const request = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++next, timer = setTimeout(() => { waiting.delete(id); reject(new Error(`MCP timeout: ${method}: ${stderr}`)); }, 90000);
  waiting.set(id, { resolve, reject, timer }); send({ id, method, params });
});
async function call(name, args) {
  const result = await request("tools/call", { name, arguments: args });
  assert.ok(!result.isError, result.content?.[0]?.text);
  return JSON.parse(result.content[0].text);
}
function local(info, path, body) {
  return new Promise((resolve, reject) => {
    const bytes = body ? Buffer.from(JSON.stringify(body)) : null;
    const request = httpRequest(info.endpoint + path, { method: body ? "POST" : "GET", headers: {
      Authorization: `Bearer ${info.token}`, ...(bytes ? { "Content-Type": "application/json", "Content-Length": bytes.length } : {}),
    } }, response => {
      const chunks = []; response.on("data", chunk => chunks.push(chunk));
      response.on("end", () => response.statusCode === 200 ? resolve(JSON.parse(Buffer.concat(chunks))) : reject(new Error(`Sidecar HTTP ${response.statusCode}`)));
    });
    request.on("error", reject); request.setTimeout(60000, () => request.destroy(new Error("Sidecar timeout"))); request.end(bytes);
  });
}
try {
  await request("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "recall-smoke", version: "1" } });
  send({ method: "notifications/initialized" });
  const receipt = await call("create-task", { title: "统一侧栏抽屉交互", agentWorkspace: { path: workspace },
    initialRun: { agentConversation: [conversation], title: "恢复抽屉默认宽度", goal: "抽屉默认占窗口一半，拖动后双击分隔线可以恢复默认宽度。", solution: "侧栏组件记录默认比例，双击触发复位。",
      validationPlan: { items: [{ title: "双击恢复宽度", steps: "拖窄抽屉后双击分隔线", expected: "恢复窗口一半宽度" }] } } });
  await call("finish-task-run", { taskId: receipt.taskId, runId: receipt.runId, expectedRevision: receipt.revision,
    agentConversation: [conversation], conclusion: "侧栏组件已接入双击复位；实际拖窄后双击，抽屉恢复半窗宽度。", changedFiles: [],
    verification: { planRevision: receipt.planRevision, stage: "local", target: "fixture", environment: "isolated model integration",
      results: [{ criterionId: receipt.criterionIds[0], result: "passed", evidence: "隔离样例记录：拖窄后双击分隔线，抽屉恢复窗口一半宽度。" }] } });
  let info, status;
  for (let attempt = 0; attempt < 120; attempt++) {
    try {
      info = JSON.parse(await readFile(join(data, "tasks/recall/runtime.json"), "utf8"));
      status = await local(info, "/health");
      if (status.ready === 1) break;
    } catch {}
    assert.ok(attempt < 119, "finish did not produce an indexed Run");
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
  const result = await call("find-task-runs", { workspace, query: "把侧栏面板拖窄后，双击可以还原一半窗口的比例" });
  assert.equal(result.runs[0]?.runId, receipt.runId);
  assert.equal(result.runs[0]?.isCurrentRun, true);
  assert.equal(result.recall.state, "ready");
  for (const route of ["semantic", "graph", "fulltext"]) assert.ok(result.runs[0].recall.evidence.some(item => item.route === route), `missing ${route}`);
  const graph = await local(info, "/graph", {});
  assert.ok(graph.nodes.length > 0);
  const output = join(root, "artifacts/task-recall-sidecar-qa"); await mkdir(output, { recursive: true });
  await writeFile(join(output, "mcp-recall-report.json"), JSON.stringify({ success: true, finishTriggered: true, result, graphNodes: graph.nodes.length }, null, 2));
  console.log(JSON.stringify({ success: true, finishTriggered: true, routes: result.recall.routes, runId: receipt.runId, graphNodes: graph.nodes.length }));
} finally {
  child.stdin.end();
  await new Promise(resolve => { if (child.exitCode !== null) return resolve(); const timer = setTimeout(() => child.kill(), 5000); child.once("exit", () => { clearTimeout(timer); resolve(); }); });
  for (const entry of waiting.values()) clearTimeout(entry.timer);
  await rm(temporary, { recursive: true, force: true });
}
