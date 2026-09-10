<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import LineageMessages from "./LineageMessages.svelte";
  import type { ExtractionJob, DirtyConversation } from "./taskLineage";
  import { lineageApi, creationLabel, episodePositions, reachable, rootThread, taskStatus, type Episode, type LineageMessage, type LineageOptions, type LineageSnapshot, type LineageTask, type LineageThread, type WorkspaceFacts } from "./taskLineage";
  let options: LineageOptions | null = null;
  let providerId = "", error = "", busy = "", search = "", mode: "conversation" | "task" = "task", view: "conversation" | "task" = "task", diagram: "graph" | "swimlane" = "graph";
  let data: LineageSnapshot = { threads: [], tasks: [], pendingMessages: 0, diagnostics: [] };
  let selectedThreadId = "", selectedTaskId = "", selectedEpisodeId = "", hovered = "";
  let expanded = new Set<string>(), statuses: Record<string, string> = {}, facts: WorkspaceFacts | null = null;
  let nodeFacts: Record<string, WorkspaceFacts> = {};
  let messages: LineageMessage[] = [], messageBusy = false;
  let requestGeneration = 0, disposed = false;
  let conversationAnchor = "";
  let organizeScope = "all";
  let jobs: ExtractionJob[] = [], dirty: DirtyConversation[] = [], taskWorkspace = "";
  $: extracting = jobs.some(job => ["queued", "running"].includes(job.state));
  async function refreshManagement() {
    const provider = providerId;
    const [nextJobs, nextDirty] = await Promise.all([lineageApi.jobs(provider), lineageApi.dirty(provider)]);
    if (disposed || provider !== providerId) return;
    jobs = nextJobs; dirty = nextDirty;
  }
  function dirtyLabel(id: string, states: DirtyConversation[]) { const state = states.find(s => s.threadId === id); return state ? ({clean: "已分析", dirty: `待抽取 ${state.pendingMessages}`, extracting: "抽取中", error: "抽取失败"})[state.state] ?? "" : ""; }
  const jobLabel = (state: string) => ({queued: "排队中", running: "抽取中", completed: "已完成", failed: "失败", interrupted: "已中断"})[state] ?? state;
  async function allocateWorkspace() { if (task) await operation("申请工作区", async () => { taskWorkspace = (await lineageApi.allocate(providerId, task!.id)).path; }); }
  $: selectedThread = data.threads.find(t => t.id === selectedThreadId);
  $: task = data.tasks.find(t => t.id === selectedTaskId);
  $: episode = task?.episodes.find(e => e.id === selectedEpisodeId);
  $: inspectorThread = data.threads.find(t => t.id === (view === "task" ? episode?.threadId : selectedThreadId));
  $: rootId = rootThread(selectedThreadId, data.threads);
  $: relatedTasks = data.tasks.filter(t => t.rootThreadId === rootId);
  $: roots = data.threads.filter(t => !t.parentId || !data.threads.some(p => p.id === t.parentId));
  $: visibleRoots = roots.filter(thread => treeMatches(thread, search, data.threads)).sort((a, b) => a.workspace.localeCompare(b.workspace));
  $: visibleTasks = data.tasks.filter(t => `${t.title} ${t.detail}`.toLowerCase().includes(search.toLowerCase()));
  $: path = task && hovered ? reachable(task, hovered) : new Set<string>();
  $: hoverThread = task?.episodes.find(e => e.id === hovered)?.threadId;
  $: episodeMessages = episode ? messages.filter(m => episode.evidenceIds.includes(m.evidence.eventId)) : [];
  $: lanes = task ? [...new Set(task.episodes.map(e => e.threadId))] : [];
  $: orderedEpisodes = task ? [...task.episodes].sort((a, b) => (a.startedAt ?? "").localeCompare(b.startedAt ?? "")) : [];
  $: positions = episodePositions(task, diagram);
  $: canvasWidth = Math.max(530, ...Object.values(positions).map(p => p.x + 242));
  $: canvasHeight = Math.max(350, lanes.length * 155 + 45);

  function children(id: string): LineageThread[] { return data.threads.filter(t => t.parentId === id); }
  function treeMatches(thread: LineageThread, query: string, threads: LineageThread[]): boolean {
    const term = query.toLowerCase();
    return !term || threads.some(t => rootThread(t.id, threads) === thread.id && t.title.toLowerCase().includes(term));
  }
  function toggle(id: string) { expanded = new Set(expanded); if (expanded.has(id)) expanded.delete(id); else expanded.add(id); }
  async function operation(label: string, callback: () => Promise<void>) {
    if (busy) return;
    busy = label; error = "";
    try { await callback(); } catch (e) { error = String(e); } finally { busy = ""; }
  }
  async function refreshOptions() { await operation("刷新来源", async () => { options = await lineageApi.options(); if (!providerId) providerId = options.sources[0]?.id ?? ""; }); }
  function accept(snapshot: LineageSnapshot) {
    data = snapshot;
    if (selectedTaskId && !data.tasks.some(t => t.id === selectedTaskId)) selectedTaskId = "";
    if (!data.threads.some(t => t.id === selectedThreadId)) selectedThreadId = data.threads[0]?.id ?? "";
    if (view === "task" && !selectedTaskId && data.tasks.length) void selectTask(data.tasks[0]);
  }
  async function switchSource() {
    jobs = []; dirty = []; taskWorkspace = "";
    ++requestGeneration; selectedThreadId = ""; selectedTaskId = ""; selectedEpisodeId = ""; messages = []; facts = null; statuses = {};
    nodeFacts = {};
    await operation("读取任务", async () => { accept(await lineageApi.snapshot(providerId)); await refreshManagement(); });
    if (view === "conversation" && selectedThreadId) await selectThread(selectedThreadId);
  }
  async function scan() {
    await operation("扫描记录", async () => { accept(await lineageApi.scan(providerId)); await refreshManagement(); });
    if (inspectorThread) await loadThread(inspectorThread.id);
  }
  async function extract() {
    await operation("提交整理", async () => { const job = await lineageApi.trigger(providerId, organizeScope === "all" ? null : selectedThreadId || null); jobs = [job, ...jobs.filter(j => j.id !== job.id)]; await refreshManagement(); });
  }
  async function loadThread(id: string, refresh = false) {
    const generation = ++requestGeneration; if (!refresh) { messageBusy = true; messages = []; facts = null; }
    const results = await Promise.allSettled([lineageApi.messages(providerId, id), lineageApi.workspace(providerId, id), lineageApi.status(providerId, id)]);
    if (disposed || generation !== requestGeneration) return;
    messageBusy = false;
    if (results[0].status === "fulfilled") messages = results[0].value; else error = String(results[0].reason);
    if (results[1].status === "fulfilled") { facts = results[1].value; nodeFacts = { ...nodeFacts, [id]: facts }; }
    statuses = { ...statuses, [id]: results[2].status === "fulfilled" ? results[2].value : "unknown" };
  }
  async function selectThread(id: string, anchor = "") { selectedThreadId = id; conversationAnchor = anchor; view = "conversation"; selectedEpisodeId = ""; await loadThread(id); }
  async function selectTask(selected: LineageTask) {
    taskWorkspace = "";
    selectedTaskId = selected.id; view = "task"; selectedThreadId = selected.rootThreadId;
    const first = selected.episodes[0]; selectedEpisodeId = first?.id ?? "";
    if (first) await loadThread(first.threadId);
    // Inspect only the threads of the selected task, never the entire catalog.
    for (const id of [...new Set(selected.episodes.map(e => e.threadId))]) {
      if (disposed || selectedTaskId !== selected.id) break;
      try { statuses = { ...statuses, [id]: await lineageApi.status(providerId, id) }; } catch { statuses = { ...statuses, [id]: "unknown" }; }
      try { nodeFacts = { ...nodeFacts, [id]: await lineageApi.workspace(providerId, id) }; } catch { /* The node explicitly shows unknown facts. */ }
    }
  }
  async function selectEpisode(selected: Episode) { selectedEpisodeId = selected.id; await loadThread(selected.threadId); }
  async function complete() {
    if (!task) return;
    const current = task;
    await operation("保存验收", async () => { const saved = await lineageApi.complete(providerId, current, !current.manualCompletion); data = { ...data, tasks: data.tasks.map(t => t.id === saved.id ? saved : t) }; });
  }
  async function send(text: string) {
    const id = inspectorThread?.id; if (!id) throw new Error("请先选择线程");
    if (busy) throw new Error("请等待当前操作完成");
    if (["running", "waiting-approval"].includes(await lineageApi.status(providerId, id))) throw new Error("当前会话正在执行或等待审批，请先处理后再发送");
    const provider = providerId, currentTask = view === "task" ? task : undefined;
    busy = "发送消息";
    try { await lineageApi.send(provider, id, text); } finally { busy = ""; }
    statuses = { ...statuses, [id]: "running" };
    if (currentTask?.manualCompletion) {
      try { const saved = await lineageApi.complete(provider, currentTask, false); data = { ...data, tasks: data.tasks.map(t => t.id === saved.id ? saved : t) }; }
      catch (e) { error = `消息已发送，任务重新打开状态需要刷新：${String(e)}`; }
    }
    void scan();
  }
  function statusLabel(id: string, currentStatuses: Record<string, string>) { return ({ running: "运行中", "waiting-approval": "等待审批", "waiting-user-input": "等待输入", idle: "已结束", archived: "已归档", error: "执行错误", unknown: "状态待确认" })[currentStatuses[id]] ?? "状态待确认"; }
  onMount(() => {
    let unlisten: (() => void) | null = null;
    void operation("加载配置", async () => { options = await lineageApi.options(); providerId = options.sources[0]?.id ?? ""; if (providerId) { accept(await lineageApi.snapshot(providerId)); await refreshManagement(); } }).then(() => { if (view === "conversation" && selectedThreadId) void loadThread(selectedThreadId); });
    let polling = false;
    const poll = setInterval(async () => {
      if (disposed || busy || polling || !providerId) return;
      polling = true;
      try { const wasExtracting = extracting; await refreshManagement(); if (wasExtracting && !extracting) accept(await lineageApi.snapshot(providerId)); }
      catch (e) { error = String(e); } finally { polling = false; }
    }, 3000);
    void listen<{ providerId: string; error: string | null }>("task-lineage-updated", event => {
      if (disposed || busy || event.payload.providerId !== providerId) return;
      void operation("同步后台结果", async () => {
        accept(await lineageApi.snapshot(providerId)); await refreshManagement(); options = await lineageApi.options();
        if (inspectorThread) await loadThread(inspectorThread.id, true);
        const selected = task;
        if (selected) for (const id of [...new Set(selected.episodes.map(e => e.threadId))]) {
          if (disposed || selectedTaskId !== selected.id) break;
          try { statuses = { ...statuses, [id]: await lineageApi.status(providerId, id) }; }
          catch { statuses = { ...statuses, [id]: "unknown" }; }
        }
        if (event.payload.error) error = event.payload.error;
      });
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(e => error = String(e));
    return () => { disposed = true; ++requestGeneration; clearInterval(poll); unlisten?.(); };
  });
</script>

<div class="lineage">
  <div class="controls">
    <label>Codex 来源<select bind:value={providerId} on:change={switchSource} disabled={!!busy}>{#each options?.sources ?? [] as source}<option value={source.id}>{source.name}</option>{/each}</select></label>
    <button on:click={refreshOptions} disabled={!!busy}>刷新来源</button>
    <button on:click={scan} disabled={!!busy || !providerId}>扫描记录</button>
    <label>整理范围<select aria-label="整理范围" bind:value={organizeScope} disabled={!!busy || extracting}><option value="all">全部近期历史</option><option value="conversation" disabled={!selectedThreadId}>当前关联对话</option></select></label>
    <button on:click={extract} disabled={!!busy || extracting || !providerId || !options?.claudeExecutable}>{extracting ? "整理中…" : "手动整理历史"}</button>
    <span role="status">{busy || `${data.pendingMessages} 条近 48 小时消息待分析`}</span>
  </div>
  <p class="hint">仅抽取最近 48 小时的用户消息与 AI 正文，排除工具执行及无可靠时间戳的内容。每批预算上限 ${options?.sources.find(s => s.id === providerId)?.extraction?.budgetUsd ?? options?.budgetUsd ?? 0.25}。后台更新在应用运行期间执行，连续抽取处理开启时选定的对话及派生线程。</p>
  <details class="diagnostics"><summary>整理历史 · {jobs.length} 次运行 · {dirty.filter(s => s.state === "dirty").length} 个会话待整理</summary>{#if !jobs.length}<p>暂无整理记录。手动与自动运行的结果会显示在这里。</p>{/if}{#each jobs as job}<p>{new Date(job.createdAt).toLocaleString()} · {job.id.startsWith("scheduled-") ? "自动" : "手动"} · {jobLabel(job.state)} · {data.threads.find(t => t.id === job.threadId)?.title ?? "全部近期历史"}{#if job.finishedAt} · 耗时 {Math.max(0, Math.round((job.finishedAt-job.createdAt)/1000))} 秒{/if}{#if job.error} · {job.error}{/if}</p>{/each}</details>
  <p class="hint">每次整理处理一批近期正文，保留已整理进度；自动提取按设置的间隔继续处理。Agent 配置位于右上角设置。</p>
  {#if data.lastExtraction}<p class="hint">最近实际模型：{Object.keys(data.lastExtraction.modelUsage ?? {}).join("、") || "CLI 未返回"} · 仅抽取用户消息与 AI 正文，不包含工具执行。</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !options?.claudeExecutable && options}<p class="hint">请先在“连接”中配置可用的 Claude 运行时。仍可扫描和查看原始记录。</p>{/if}
  {#if data.diagnostics.length}<details class="diagnostics"><summary>{data.diagnostics.length} 项记录读取或抽取问题</summary>{#each data.diagnostics as diagnostic}<p>{diagnostic}</p>{/each}</details>{/if}
  <div class="workspace">
    <aside class="navigation" aria-label="对话与任务导航">
      <h3>任务管理</h3>
      <input type="search" bind:value={search} placeholder="搜索对话或任务" aria-label="搜索对话或任务" />
      <div class="segmented"><button class:active={mode === "conversation"} on:click={() => mode = "conversation"}>对话</button><button class:active={mode === "task"} on:click={() => mode = "task"}>任务</button></div>
      <div class="nav-list">
        {#if mode === "conversation"}
          {#each visibleRoots as thread, index (thread.id)}
            {#if index === 0 || visibleRoots[index - 1].workspace !== thread.workspace}<div class="workspace-label" title={thread.workspace}>{thread.workspace.split(/[\\/]/).pop() || "工作区未知"}</div>{/if}
            <div class="tree-row"><button class="expand" aria-label={`展开 ${thread.title}`} aria-expanded={expanded.has(thread.id)} on:click={() => toggle(thread.id)}>{children(thread.id).length ? expanded.has(thread.id) ? "▾" : "▸" : "·"}</button><button class="thread" class:active={selectedThreadId === thread.id} on:click={() => selectThread(thread.id)}><strong>{thread.title}</strong><small>{creationLabel(thread.creationKind)} · {data.tasks.filter(t => t.rootThreadId === thread.id).length} 项任务</small></button></div>
            {#if expanded.has(thread.id) || search}
              {#each data.threads.filter(t => t.id !== thread.id && rootThread(t.id, data.threads) === thread.id) as child (child.id)}<button class="thread child" class:active={selectedThreadId === child.id} on:click={() => selectThread(child.id)}><strong>{child.title}</strong><small>{creationLabel(child.creationKind)}</small></button>{/each}
            {/if}
          {/each}
        {:else}
          {#each visibleTasks as item (item.id)}<button class="thread" class:active={selectedTaskId === item.id} on:click={() => selectTask(item)}><strong>{item.title}</strong><small>{taskStatus(item, statuses)}</small></button>{/each}
        {/if}
        {#if !data.threads.length}<p class="empty">选择 Codex 来源并扫描记录，开始建立对话树。</p>{/if}
      </div>
      <footer>{data.threads.length} 个线程 · {data.tasks.length} 项任务</footer>
    </aside>
    <section class="center" aria-label="当前工作视角">
      <div class="center-toolbar"><div class="segmented"><button class:active={view === "conversation"} on:click={() => selectedThreadId && selectThread(selectedThreadId)}>对话</button>{#if selectedThread?.createdBy === "human" && !selectedThread.parentId}<button class:active={view === "task"} on:click={() => relatedTasks[0] ? selectTask(relatedTasks[0]) : view = "task"}>任务</button>{/if}</div><span>{selectedThread?.workspace.split(/[\\/]/).pop() ?? ""}</span></div>
      {#if view === "conversation"}
        <header class="title"><small>{creationLabel(selectedThread?.creationKind ?? "unknown")}</small><h3>{selectedThread?.title ?? "选择一个对话"}</h3>{#if selectedThread}<span>{statusLabel(selectedThread.id, statuses)} · {dirtyLabel(selectedThread.id, dirty)}</span>{/if}</header>
        <LineageMessages {messages} contextId={selectedThreadId} anchorEventId={conversationAnchor} title={selectedThread?.title ?? "对话消息"} busy={messageBusy} onSend={selectedThread ? send : null} />
      {:else if task}
        <div class="task-picker">{#each relatedTasks as item}<button class:active={item.id === task.id} on:click={() => selectTask(item)}>{item.title}</button>{/each}</div>
        <div class="task-picker"><button disabled={!!busy} on:click={allocateWorkspace}>申请任务工作区</button></div>
        {#if taskWorkspace}<p class="hint" role="status">任务工作区：{taskWorkspace}</p>{/if}
        <header class="title"><div class="task-heading"><h3>{task.title}</h3><button disabled={!!busy || (!task.manualCompletion && taskStatus(task, statuses) !== "待验收")} on:click={complete}>{task.manualCompletion ? "重新打开" : "标记已完成"}</button></div><span>{taskStatus(task, statuses)}</span><p>{task.detail}</p></header>
        <div class="diagram-toolbar"><div class="segmented"><button class:active={diagram === "graph"} on:click={() => diagram = "graph"}>自由图</button><button class:active={diagram === "swimlane"} on:click={() => diagram = "swimlane"}>时间泳道</button></div><small>悬停或聚焦查看路径 · 点击回溯消息</small></div>
        <div class="canvas-scroll"><div class="canvas" style:width={`${canvasWidth}px`} style:height={`${canvasHeight}px`}>
          {#if diagram === "swimlane"}{#each lanes as lane, index}<div class="lane" style:top={`${index * 155}px`} style:width={`${canvasWidth}px`}><small>{creationLabel(data.threads.find(t => t.id === lane)?.creationKind ?? "unknown")} · {data.threads.find(t => t.id === lane)?.title ?? lane}</small></div>{/each}{/if}
          <svg width={canvasWidth} height={canvasHeight} aria-hidden="true"><defs><marker id="lineage-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor" /></marker></defs>{#each task.edges as edge}{#if positions[edge.from] && positions[edge.to]}<path class:highlight={path.has(edge.from) && path.has(edge.to)} d={`M${positions[edge.from].x + 214},${positions[edge.from].y + 50} C${positions[edge.from].x + 250},${positions[edge.from].y + 50} ${positions[edge.to].x - 35},${positions[edge.to].y + 50} ${positions[edge.to].x},${positions[edge.to].y + 50}`} fill="none" stroke="currentColor" stroke-width="2" marker-end="url(#lineage-arrow)" />{/if}{/each}</svg>
          {#each orderedEpisodes as node (node.id)}<button class="node" class:selected={node.id === selectedEpisodeId} class:same-thread={!!hovered && node.threadId === hoverThread} class:dim={!!hovered && !path.has(node.id) && node.threadId !== hoverThread} style:left={`${positions[node.id].x}px`} style:top={`${positions[node.id].y}px`} on:mouseenter={() => hovered = node.id} on:mouseleave={() => hovered = ""} on:focus={() => hovered = node.id} on:blur={() => hovered = ""} on:click={() => selectEpisode(node)}><strong>{node.title}</strong><small>{creationLabel(data.threads.find(t => t.id === node.threadId)?.creationKind ?? "unknown")} · {statusLabel(node.threadId, statuses)}</small><time>{node.startedAt ? new Date(node.startedAt).toLocaleString() : "时间未知"}</time><span class="node-foot">{nodeFacts[node.threadId]?.workMode === "worktree" ? "独立 worktree" : nodeFacts[node.threadId]?.workMode === "main" ? "主工作区" : "工作区未知"} · {nodeFacts[node.threadId]?.commitState === "clean" ? "干净" : nodeFacts[node.threadId]?.commitState === "uncommitted" ? "未提交" : "提交未知"} · {nodeFacts[node.threadId]?.syncState === "synced" ? "已同步" : nodeFacts[node.threadId]?.syncState === "notRequired" ? "无需同步" : "同步未知"}</span></button>{/each}
        </div></div>
      {:else}<p class="empty">暂无任务，点击“手动整理历史”开始。</p>{/if}
    </section>
    <aside class="inspector" aria-label="事实与原始消息">
      <header class="title"><small>{view === "task" ? "节点信息" : "对话信息"}</small><h3>{view === "task" ? episode?.title ?? "选择执行节点" : selectedThread?.title ?? "可核验事实"}</h3>{#if view === "task" && episode}<button on:click={() => selectThread(episode!.threadId, episode!.evidenceIds[0])}>打开原始对话</button>{/if}</header>
      {#if facts}<dl><dt>当前工作区</dt><dd>{facts.workspace}</dd><dt>分支</dt><dd>{facts.branch ?? "未知 / detached HEAD"}</dd><dt>当前提交</dt><dd>{facts.head ?? "未知"}</dd><dt>工作区状态</dt><dd>{facts.commitState === "clean" ? "已提交 · 工作区干净" : facts.commitState === "uncommitted" ? "有未提交变化" : "未知"}</dd><dt>与主工作区同步</dt><dd>{facts.syncState === "synced" ? "已同步" : facts.syncState === "notRequired" ? "无需同步" : "尚未证实"}</dd></dl><p class="hint">以上为当前 Git 状态，不代表历史节点完成时的状态。</p>{#if facts.diagnostic}<p class="hint">{facts.diagnostic}</p>{/if}{/if}
      {#if view === "task" && episode}<LineageMessages compact messages={episodeMessages} contextId={episode.threadId} title="执行节点原始消息" busy={messageBusy} onSend={send} />{:else}<div class="associated"><h4>关联任务</h4>{#each relatedTasks as item}<button on:click={() => selectTask(item)}>{item.title}<small>{taskStatus(item, statuses)}</small></button>{/each}{#if !relatedTasks.length}<p class="empty">暂无关联任务</p>{/if}</div>{/if}
    </aside>
  </div>
</div>

<style>
  .lineage{container-type:inline-size;min-width:0;font-family:var(--font-family-ui);color:var(--app-text)}button{border:1px solid var(--app-border);border-radius:6px;padding:6px 10px;background:var(--app-surface);color:var(--app-text);font-size:12px}.controls{display:flex;align-items:end;gap:10px;flex-wrap:wrap;padding-bottom:6px}.controls label{font-size:11px}.controls select{display:block;max-width:180px;min-height:30px}.controls span{font-size:12px;color:var(--app-muted)}.hint{font-size:11px;color:var(--app-muted);margin:6px 0 12px;overflow-wrap:anywhere}.error{color:var(--color-main-danger-text);overflow-wrap:anywhere}.workspace{display:grid;grid-template-columns:220px minmax(0,1fr) 300px;height:calc(100vh - 215px);min-height:550px;border:1px solid var(--color-main-divider);border-radius:10px;background:var(--color-main-canvas);overflow:hidden}.navigation,.center,.inspector{min-width:0;min-height:0;display:flex;flex-direction:column}.navigation{padding:14px 8px 8px;border-right:1px solid var(--color-main-divider)}h3{font-size:15px;margin:0 0 10px;overflow-wrap:anywhere}.navigation h3{padding-left:8px}.navigation input{width:100%;min-width:0}.segmented{display:flex;gap:3px}.segmented button{flex:1;padding:5px 10px;background:transparent;border:0;font-size:12px}.active{background:var(--color-main-selected)!important;color:var(--color-main-selected-text)!important}.navigation>.segmented{margin:10px 0}.nav-list{flex:1;overflow:auto;min-height:0}.workspace-label{padding:14px 8px 5px;font-size:11px;font-weight:600;color:var(--app-muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.tree-row{display:flex}.thread{display:flex;flex-direction:column;gap:5px;width:100%;text-align:left;border:0;background:transparent;padding:10px 8px;min-width:0}.thread strong{font-size:12px;font-weight:500;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}.thread small{font-size:10px;color:var(--app-muted)}.expand{padding:0 3px;border:0;background:transparent;min-width:20px}.child{padding-left:28px;border-left:1px solid var(--color-main-divider)}footer{font-size:11px;padding:10px 5px;color:var(--app-muted)}.center-toolbar{display:flex;align-items:center;justify-content:space-between;padding:10px 16px;border-bottom:1px solid var(--color-main-divider)}.center-toolbar span{font-size:11px}.title{padding:16px;border-bottom:1px solid var(--color-main-divider);flex:none}.title small,.title span{font-size:11px;color:var(--app-muted)}.title h3{margin:6px 0}.title p{font-size:12px;margin:7px 0 0}.task-heading{display:flex;align-items:start;gap:10px;justify-content:space-between}.task-heading button{white-space:nowrap;font-size:11px}.task-picker{display:flex;gap:6px;overflow:auto;padding:10px 12px}.task-picker button{font-size:11px;white-space:nowrap}.diagram-toolbar{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:10px}.diagram-toolbar small{font-size:10px;color:var(--app-muted)}.canvas-scroll{overflow:auto;flex:1;min-height:220px;background:var(--app-bg);overscroll-behavior:contain}.canvas{position:relative}.canvas svg{position:absolute;inset:0;color:var(--app-muted);pointer-events:none}.canvas path.highlight{color:var(--blue-9)}.node{position:absolute;width:214px;min-height:105px;text-align:left;padding:12px;display:flex;flex-direction:column;gap:7px;background:var(--color-main-canvas);border:1px solid var(--color-main-outline);border-radius:9px;box-shadow:0 2px 6px #0000000a}.node strong{font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:100%}.node small,.node time,.node-foot{font-size:10px;color:var(--app-muted)}.node-foot{border-top:1px solid var(--color-main-divider);padding-top:6px;width:100%}.node.selected{outline:2px solid var(--blue-9);outline-offset:1px}.node.same-thread{border-color:var(--purple-9)}.node.dim{opacity:.4}.lane{position:absolute;height:155px;border-bottom:1px solid var(--color-main-divider);padding:5px 10px}.lane small{font-size:10px;color:var(--app-muted);display:block;max-width:350px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}.inspector{border-left:1px solid var(--color-main-divider);overflow:auto}.inspector :global(.message-panel){flex:1 0 320px;min-height:320px}.inspector>.hint{padding:0 14px}.inspector dl{padding:12px 14px;margin:0;font-size:11px}.inspector dt{color:var(--app-muted);margin-top:8px}.inspector dd{margin:3px 0;overflow-wrap:anywhere;font-family:var(--font-family-code)}.associated{padding:14px}.associated h4{font-size:12px;margin:0 0 10px}.associated button{display:flex;flex-direction:column;width:100%;text-align:left;gap:5px;font-size:12px;margin-bottom:5px}.associated small,.empty{color:var(--app-muted)}.empty{font-size:12px;padding:16px}.diagnostics{font-size:12px;margin-bottom:10px}button:focus-visible,input:focus-visible,select:focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}
  @container(max-width:1050px){.workspace{grid-template-columns:210px minmax(0,1fr);height:auto;min-height:620px}.navigation{grid-row:1 / 3;max-height:850px}.center{height:570px}.inspector{grid-column:2;max-height:420px;border-top:1px solid var(--color-main-divider);border-left:0}.diagram-toolbar small{display:none}}
  @container(max-width:650px){.workspace{display:flex;flex-direction:column}.navigation{max-height:260px;border-bottom:1px solid var(--color-main-divider)}.center{min-height:520px}.inspector{max-height:420px}.controls{gap:6px}.controls select{max-width:135px}.title{padding:12px}}
</style>
