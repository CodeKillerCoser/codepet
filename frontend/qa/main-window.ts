import {taskLedgerOverflow} from './taskLedgerOverflow';
// Browser-only QA fixture. This entry is excluded from production build inputs.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { mount } from "svelte";
import App from "../App.svelte";
import ExtractionHistory from "../lib/settings/ExtractionHistory.svelte";
import { frontendBuild } from "../lib/webcontent";
import { defaultRunningBubbleSettings } from "../lib/theme";
import "../styles.css";
import "../main-window.css";
import type { LineageSnapshot, LineageMessage } from "../lib/taskLineage";
let watch = { revision: 0, enabled: false, extract: false, threadId: null as string | null, model: "haiku", remainingJobs: 5, lastError: null };
let config = { revision: 0, harness: "claude", harnessInstanceId: null, model: "haiku", reasoningEffort: "low", automatic: false, skill: "extract-tasks", prompt: "", budgetUsd: 0.25, timeoutSeconds: 90, intervalSeconds: 60, debounceSeconds: 20 };
let jobs: any[] = [];
const timestamp = "2026-09-10T04:00:00Z";
const data: LineageSnapshot = {
  threads: [
    { id: "main", title: "构建 Codex 任务谱系管理", workspace: "D:/Projects/CodePet", createdBy: "human", creationKind: "main", parentId: null, sourceFile: "/fixture/main.jsonl", timestamp },
    { id: "child", title: "验证增量读取与证据边界", workspace: "D:/Worktrees/CodePet-reader", createdBy: "agent", creationKind: "newThread", parentId: "main", sourceFile: "/fixture/child.jsonl", timestamp },
  ], tasks: [{ id: "task", revision: 1, rootThreadId: "main", title: "任务谱系第一版", detail: "从原始对话识别可验收任务，追踪跨线程执行过程，并回到消息继续处理。", manualCompletion: false,
    episodes: [
      { id: "start", threadId: "main", title: "确定任务抽取边界", evidenceIds: ["m1", "m2"], startedAt: timestamp, endedAt: timestamp },
      { id: "read", threadId: "child", title: "验证增量读取", evidenceIds: ["c1", "c2"], startedAt: "2026-09-10T04:10:00Z", endedAt: "2026-09-10T04:30:00Z" },
      { id: "finish", threadId: "main", title: "整合并请求验收", evidenceIds: ["m3"], startedAt: "2026-09-10T04:40:00Z", endedAt: "2026-09-10T04:50:00Z" },
    ], edges: [{ from: "start", to: "read", evidenceIds: ["m2"] }, { from: "read", to: "finish", evidenceIds: ["m3"] }] }], pendingMessages: 0, diagnostics: [],
  lastExtraction: { requestedModel: "haiku", modelUsage: { "fixture-low-cost-model": {} }, reportedCostUsd: null },
};
const history: LineageMessage[] = [
  ["m1", "main", "user", "任务抽取只把用户消息和 AI 正文作为对象，工具执行不要。"],
  ["m2", "main", "assistant", "已将这一边界写入来源解析和执行器校验。下一步验证增量追加与恢复。"],
  ["c1", "child", "user", "验证半行追加、同长度改写和工具文本排除。"],
  ["c2", "child", "assistant", "三项来源回归测试已通过，证据位置保持稳定。"],
  ["m3", "main", "assistant", "代码已经整合，等待人工确认第一版验收。"],
].map(([id, threadId, role, text], index) => ({ evidence: { eventId: id, file: `/fixture/${threadId}.jsonl`, byteOffset: index * 200, generation: 0 }, threadId, role, text, timestamp, turnId: id }));
mockWindows("main");
let appSettings: any = {
  data: {}, appearance: {theme: new URLSearchParams(location.search).get("theme") ?? "light", runningBubble: {...defaultRunningBubbleSettings}},
  pet: {selectedPetId: "default", kind: "builtin", scale: 3, opacity: 1, whipReactionSound: "none"},
  petLibrary: {selectedPetId: "default", dataDirectory: "/fixture/pets", pets: [{id:"default", name:"橘猫", kind:"builtin"}]},
  notifications: {sound:"blip", ringOnPermission:true, ringOnFailure:true, ringOnDone:true, repeatSeconds:30, quietHoursEnabled:false, quietHoursStart:"22:00", quietHoursEnd:"08:00"},
  providerPlugins: {directories: []}, updates: {},
};
const settingsWrites: any[] = [];
Object.assign(window, {
  __settingsWrites: settingsWrites,
  __getAppSettings: () => structuredClone(appSettings),
  __pairingCancellations: [] as string[],
});
let launchAtLogin = false;
let pairingId = 0;
const pairingExpiry = Date.now() + 120_000;
const pairingQr = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><path d="M20 20h60v60H20zm100 0h60v60h-60zM20 120h60v60H20zm100 0h60v60h-60z"/></svg>');
const runtime = {providerId:"dev.codepet.claude", displayName:"Claude", status:"ready", source:"configured", resolvedExecutable:"D:/Software/Claude/claude.exe", configuredExecutable:"D:/Software/Claude/claude.exe", version:"2.1.263", installed:[]};
// Inventory scenarios exercise visibility without accessing installed tools.
const runtimeCase = new URLSearchParams(location.search).get("runtimeCase");
const missingRuntime = (providerId: string, displayName: string) => ({
  providerId, displayName, status: "unavailable", resolvedExecutable: null,
  configuredExecutable: null, source: null, version: null, installed: [],
  diagnostic: { code: "runtime-not-found", message: "No compatible local runtime" },
});
const absentRuntimes = [missingRuntime("dev.codepet.claude", "Claude"), missingRuntime("dev.codepet.opencode", "OpenCode")];
const codexPath = "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";
let runtimeInventory = runtimeCase === "missing" ? [...absentRuntimes, missingRuntime("dev.codepet.codex", "Codex")]
  : runtimeCase === "chatgpt" ? [...absentRuntimes, { ...runtime, providerId: "dev.codepet.codex", displayName: "Codex", source: "macos-application", configuredExecutable: null,
    resolvedExecutable: codexPath, version: "0.166.0-alpha.5.1", installed: [{executablePath: codexPath, version: "0.166.0-alpha.5.1", source: "macos-application"}] }]
  : runtimeCase === "cards" ? [runtime,
    { ...runtime, providerId: "dev.codepet.codex", displayName: "Codex", resolvedExecutable: codexPath, installed: [{executablePath: codexPath, version: "0.167.0", source: "macos-application"}] },
    { ...runtime, providerId: "dev.codepet.opencode", displayName: "OpenCode", resolvedExecutable: "/opt/homebrew/lib/node_modules/@opencode/cli/bin/opencode.exe", installed: [] },
    { ...runtime, providerId: "qa.notification", displayName: "消息通知", status: "not-required", resolvedExecutable: null },
    { ...runtime, providerId: "qa.agent", displayName: "本地助手", status: "not-required", resolvedExecutable: null },
  ] : [runtime];
const runtimeConnections = () => runtimeInventory.map((item) => ({providerId:item.providerId,displayName:item.displayName,icon:"https://avatars.githubusercontent.com/u/14957082?s=200&v=4",version:"0.1.0",defaultWorkspaceRoot:"/Users/wangxin/.codepet/remote_workspace/codex",connectionStatus:"online",generation:1,instances:[{id:item.providerId,status:"ready"}]}));
mockIPC((command, raw) => {
  if (command === "task_ledger_request") {
    const request = (raw as any).request;
    if (request.op === "list") return fetch('/artifacts/task-ledger-qa/native/report.json').then(r => r.json()).then(report => new URLSearchParams(location.search).has('taskOverflow') ? taskLedgerOverflow(report.taskSnapshot) : [report.taskSnapshot]);
    if (request.op === "setup") return {config: '[mcp_servers.codepet_tasks]\nurl = "http://127.0.0.1:47623/mcp"\nhttp_headers = { "X-CodePet-Agent" = "codex" }', instructions: '界面验收样例，不修改本机配置。'};
    throw new Error(`Unhandled task ledger fixture operation: ${request.op}`);
  }
  if (command === "codex_workbench_request") {
    const request = (raw as any).request;
    if (request.op === "projects") return { projects: [{ id: "qa", name: "Code Pet", roots: ["/fixture/codepet"] }], warnings: [] };
    if (request.op === "preferences") return ["qa"];
    if (request.op === "scan") return { scanId: 1, projects: [{ id: "qa", name: "Code Pet", roots: ["/fixture/codepet"] }], worktrees: [{ id: "w", projectId: "qa", projectName: "Code Pet", path: "/fixture/codepet", main: true, conversations: [{id:"c",source:"Codex",title:"检查历史对话",cwd:"/fixture/codepet",updatedAt:Date.now(),archived:false}], git: { branch:"main", head:"a1b2c3", upstream:false, ahead:0, behind:0, changes:[] } }], warnings: [], scannedAt: Date.now() };
    return null;
  }
  const permissionScenario = new URLSearchParams(location.search);
  if (command === "get_system_permission_settings") {
    if (permissionScenario.has("permissionLoadError")) throw new Error("Simulated old backend");
    return { fullDiskAccessSupported: !permissionScenario.has("permissionUnsupported"), appPath: "/Applications/Code Pet.app" };
  }
  if (command === "open_full_disk_access_settings" || command === "reveal_permission_app") {
    if (permissionScenario.has("permissionOpenError")) throw new Error("无法打开系统设置（模拟错误）");
    return;
  }
  if (command === "webcontent_info" || command === "load_latest_webcontent") {
    const scenario = new URLSearchParams(location.search).get("webcontentCase");
    if (scenario === "unsupported") throw new Error("Command not found");
    if (command === "load_latest_webcontent" && scenario === "error") throw new Error("资源校验失败，保留当前前端资源");
    if (command === "load_latest_webcontent" && scenario === "reload" && !new URLSearchParams(location.search).has("webcontent")) return { ...frontendBuild, builtAt: frontendBuild.builtAt + 1, canReload: true };
    return { ...frontendBuild, canReload: true };
  }
  if (command === "plugin:app|version") return "0.3.8-qa";
  const args = (raw ?? {}) as Record<string, any>;
  if (command === "plugin:window|outer_position") return {x:0,y:0};
  if (command === "get_app_settings") return structuredClone(appSettings);
  if (command === "update_app_settings") {
    appSettings = structuredClone(args.settings);
    settingsWrites.push(structuredClone(appSettings));
    return structuredClone(appSettings);
  }
  if (command === "list_pets") return {...appSettings.petLibrary, dataDirectory:"/fixture/pets"};
  if (command === "get_launch_at_login_enabled") return launchAtLogin;
  if (command === "set_launch_at_login_enabled") { launchAtLogin = args.enabled; return launchAtLogin; }
  if (command === "send_test_robot_notification") return "测试通知已发送";
  if (command === "path_manager_get") return {install:"/fixture/install",data:"/fixture/data",workspace:"/fixture/workspace"};
  if (command === "list_remote_clients") return permissionScenario.get("deviceCase") === "paired" ? [{
    credentialId: "qa-device", remoteClientId: "qa-phone",
    descriptor: { deviceName: "QA Phone", operatingSystem: "iOS", systemVersion: "18.0" },
    createdAt: Date.now() - 60_000, lastSeenAt: Date.now(), revokedAt: null, onlineSessionCount: 1,
  }] : [];
  if (["recent_events", "list_remote_pairing_requests"].includes(command)) return [];
  if (command === "check_app_update") {
    const scenario = permissionScenario.get("updateCase");
    if (scenario === "error") throw new Error("无法获取最新发行版：网络连接失败（模拟）");
    if (scenario === "manual" || scenario === "installError" || scenario === "automatic") return { version: "0.3.10-beta+a68ec0c", currentVersion: "0.3.9-beta", manualDownload: scenario !== "automatic" };
    return null;
  }
  if (command === "install_app_update") {
    (window as any).__updateInstallCount = ((window as any).__updateInstallCount || 0) + 1;
    if (permissionScenario.get("updateCase") === "installError") throw new Error("无法打开安装包下载地址（模拟）");
    return;
  }
  if (command === "record_perf_event") return null;
  if (command === "plugin:dialog|open") return "/fixture/provider/codepet-provider.json";
  if (command === "plugin:path|basename") return "codepet-provider.json";
  if (command === "plugin:dialog|message") return args.buttons?.OkCancelCustom?.[0] ?? "Ok";
  if (command === "remove_provider_plugin") {
    if (permissionScenario.get("removeCase") === "error") throw new Error("测试：模拟移除失败");
    runtimeInventory = runtimeInventory.filter(item => item.providerId !== args.providerId);
    void emit("provider-connection-status", runtimeConnections());
    return null;
  }
  if (command === "import_provider_plugin") {
    runtimeInventory = [...runtimeInventory, {...runtime, version:"1.0.0", providerId:"dev.example.custom", displayName:"Custom Provider", installed:[{executablePath:"/fixture/custom", version:"1.0.0", source:"configured"}], resolvedExecutable:"/fixture/custom"}];
    void emit("provider-connection-status", runtimeConnections());
    return {providerId:"dev.example.custom", displayName:"Custom Provider", loaded:true};
  }
  if (command === "provider_connection_status") return runtimeConnections();
  if (command === "list_agent_runtimes" || command === "refresh_agent_runtimes") return runtimeInventory;
  if (command === "remote_access_status") return {phase:"available",activeSessionCount:0,pairingAvailable:true,displayName:"QA desktop",diagnostic:null};
  if (command === "start_remote_pairing") return { pairingId: `qa-pair-${++pairingId}`, expiresAt: pairingExpiry, qrSvgDataUrl: pairingQr };
  if (command === "get_remote_pairing_status") return { pairingId: args.pairingId, state: "active", expiresAt: pairingExpiry, qrSvgDataUrl: pairingQr };
  if (command === "copy_remote_pairing_json") return;
  if (command === "cancel_remote_pairing") {
    (window as any).__pairingCancellations.push(args.pairingId);
    return { pairingId: args.pairingId, state: "cancelled", expiresAt: pairingExpiry };
  }
  if (command === "query_event_journal") return {events:[],cursor:0,hasMore:false,dropped:0};
  if (command === "pet_gateway_request") { const r=args.request; return {id:r.id,method:r.method,protocolVersion:r.protocolVersion,response:{status:"ok",result:{snapshot:{sources:[{id:"codex",displayName:"Codex",enabled:true,status:"receiving"},{id:"claude",displayName:"Claude Code",enabled:true,status:"installed"},{id:"opencode",displayName:"OpenCode",enabled:false,status:"disabled"}],tasks:[]}}}}; }
  if (command === "task_lineage_options") return { sources: [{ id: "local:codex", name: "Codex（本地）", agent: "codex", connectionProviderId: new URLSearchParams(location.search).has("disconnected") ? null : "codex-fixture", directory: "/fixture", watch, extraction: config }], claudeExecutable: "/fixture/claude", defaultModel: "haiku", budgetUsd: 0.25, instances: [{ id: "claude", name: "本机 Claude", harness: "claude", controls: {modelCatalog: {kind: "flat", models: [{id: "haiku", displayName: "Haiku"}]}, reasoningEffort: {options: ["low", "medium", "high"].map(id => ({id, displayName: id}))}}}], layout: {root: "/fixture/.codepair", skills: "/fixture/.codepair/skills", extractionWorkspaces: "/fixture/.codepair/workspaces/task-extraction", taskWorkspaces: "/fixture/.codepair/workspaces/tasks"} };
  if (command === "task_lineage_request") {
    const r = args.request;
    if (r.method === "tasks.settings.get") return structuredClone(config);
    if (r.method === "tasks.settings.set") { config = { ...r.config, revision: config.revision + 1 }; watch = {...watch, enabled: config.automatic, extract: config.automatic}; return structuredClone(config); }
    if (r.method === "tasks.skills") return ["extract-tasks", "reconcile-tasks"];
    if (r.method === "tasks.skill.get") return { name: r.name, text: `---\nname: ${r.name}\ndescription: Task extraction\n---\n只抽取最近 48 小时用户消息与 AI 正文。` };
    if (r.method === "tasks.dirty") return [{threadId: "main", state: "clean", revision: 1, extractedRevision: 1, pendingMessages: 0, lastChangedAt: Date.now(), lastError: null}];
    if (r.method === "tasks.jobs") return structuredClone(jobs);
    if (r.method === "tasks.extract") { const job = {id: r.requestId, requestId: r.requestId, threadId: r.threadId, state: "queued", createdAt: Date.now(), finishedAt: null, error: null}; jobs = [job]; setTimeout(() => { job.state = "completed"; }, 500); return structuredClone(job); }
    if (r.method === "tasks.workspace.request") return {taskId: r.taskId, path: "/fixture/.codepair/workspaces/tasks/task"};
    throw new Error(`Unhandled task method: ${r.method}`);
  }
  if (command === "task_lineage_watch") { watch = { ...args.config, revision: watch.revision + 1 }; return watch; }
  if (["task_lineage_snapshot", "task_lineage_scan", "task_lineage_extract"].includes(command)) return structuredClone(data);
  if (command === "task_lineage_messages") return history.filter(m => m.threadId === args.threadId);
  if (command === "task_lineage_workspace") return { workspace: data.threads.find(t => t.id === args.threadId)?.workspace, repository: "D:/Projects/CodePet", branch: "codex/task-lineage-v1", head: "a81b0c9d4e52", mainWorkspace: "D:/Projects/CodePet", workMode: args.threadId === "child" ? "worktree" : "main", commitState: "clean", syncState: args.threadId === "child" ? "synced" : "notRequired", diagnostic: null };
  if (command === "task_lineage_complete") { data.tasks[0].manualCompletion = args.completed; data.tasks[0].revision++; return structuredClone(data.tasks[0]); }
  if (command === "codepet_gateway_request") {
    const request = args.request;
    let result: any;
    if (request.method === "provider.list") result = {providers: []};
    else if (request.method === "conversation.get") result = { conversation: { status: "idle" }, items: [] };
    else if (request.method === "provider.describe") result = { capabilities: { revision: "fixture", methods: request.params.providerId === "qa.notification" ? ["observation.subscribe"] : ["turn.send"] } };
    else if (request.method === "conversation.acquireInteraction") result = { selection: {} };
    else if (request.method === "turn.send") result = { accepted: true };
    else throw new Error(`Unhandled fixture gateway method: ${request.method}`);
    return { jsonrpc: "2.0", id: request.id, result };
  }
  throw new Error(`Unhandled fixture command: ${command}`);
}, { shouldMockEvents: true });
const target = document.getElementById("app")!;
if (new URLSearchParams(location.search).has("uiHistory")) {
  target.className = "main-theme light";
  mount(ExtractionHistory, { target, props: { open: true, sourceName: "QA", jobs: [], dirty: [], threads: [] } });
} else {
  mount(App, { target });
}
