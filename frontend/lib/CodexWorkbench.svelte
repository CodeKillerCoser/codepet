<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { Search, RefreshCw, FolderOpen, Terminal, ExternalLink, X, SlidersHorizontal } from '@lucide/svelte';
  import { MergeView } from '@codemirror/merge';
  import { EditorView } from '@codemirror/view';
  import { EditorState } from '@codemirror/state';
  import TaskLedger from './TaskLedger.svelte';
  import AppSidePanel from './shell/AppSidePanel.svelte';
  import * as Dialog from './ui/dialog/index';
  import { workbenchRequest as api, activity, dirty, unpushed, filterRows, statusMatches, branchLabel, changeLabel, gitSummary, type SourceInfo, type Project, type Snapshot, type Worktree, type Change } from './codexWorkbench';
  import { workbenchSession as session } from './codexWorkbenchSession';
  const retained = session.page;
  let projects: Project[] = [], draft: string[] = [], projectSearch = '', pickerOpen = false;
  let projectInput: HTMLInputElement;
  let snapshot: Snapshot | null = null, loading = false, catalogLoading = true, error = '', catalogWarnings: string[] = [];
  let { query, filterProjects, related, period, date, status } = retained;
  let selectedId = '', archived = false;
  let progress = { checked: 0, total: 0, phase: 'history' }, rechecking = new Set<string>(), actionMessage = '', actionError = '';
  let warningDialog: HTMLDialogElement, previewOpen = false, previewElement: HTMLDivElement, previewChange: Change | null = null, previewRow: Worktree | null = null, version = 'workspace', previewError = '', previewLoading = false, previewSequence = 0;
  let sourceInfo: SourceInfo | undefined, sourceDialog: HTMLDialogElement;
  let detailOpen = false, moreOpen = false, selectedButton: HTMLButtonElement | undefined;
  let chatQuery = '', chatLimit = 30;
  let alive = true;
  let view: 'code' | 'tasks' = 'code';
  let merge: MergeView | undefined, stop: (() => void) | undefined;
  $: ({ projects, draft, snapshot, loading, catalogLoading, error, catalogWarnings, sourceInfo, progress, rechecking } = $session);
  $: if (snapshot) filterProjects = filterProjects.filter(id => snapshot!.projects.some(p => p.id === id));
  $: base = filterRows(snapshot?.worktrees ?? [], { query, projects: filterProjects, related, period, date });
  $: rows = base.filter(r => statusMatches(r, status));
  $: selected = snapshot?.worktrees.find(r => r.id === selectedId);
  $: chats = selected?.conversations.filter(c => archived || !c.archived).sort((a,b) => b.updatedAt-a.updatedAt) ?? [];
  $: filteredChats = chats.filter(c => !chatQuery.trim() || c.title.toLocaleLowerCase().includes(chatQuery.trim().toLocaleLowerCase()));
  $: unreadable = snapshot?.worktrees.filter(r => r.git.error) ?? [];
  $: warnings = [...new Set([...catalogWarnings, ...(snapshot?.warnings ?? []), ...unreadable.map(r => `${r.path}：${r.git.error}`)])];
  $: archiveCount = selected?.conversations.filter(c => c.archived).length ?? 0;
  $: changed = !!snapshot && [...draft].sort().join() !== snapshot.projects.map(p => p.id).sort().join();
  const fmt = (n: number) => n ? new Date(n).toLocaleString('zh-CN', { month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false }) : '暂无活动';
  const loadProjects = () => session.loadProjects();
  function updateProjectSelection(projectIds: string[]) {
    projectSearch = ''; void session.selectProjects(projectIds);
    void tick().then(() => projectInput?.focus({ preventScroll: true }));
  }
  function toggleProject(id: string) { updateProjectSelection(draft.includes(id) ? draft.filter(p => p !== id) : [...draft, id]); }
  function retainProjectSearchFocus(event: PointerEvent) {
    // macOS WebKit moves focus to the ancestor section before a button's click.
    // Keep this popup mounted until selection, including touch/pen activation.
    if (event.button === 0) event.preventDefault();
  }
  async function scan() {
    if (previewOpen) closePreview();
    detailOpen = false;
    selectedId = ''; chatQuery = ''; chatLimit = 30; pickerOpen = false;
    await session.scan();
  }
  function reset() { query = ''; filterProjects = []; related = false; period = 'all'; date = ''; status = 'all'; }
  function selectRow(row: Worktree, button: HTMLButtonElement) {
    selectedId = row.id; selectedButton = button; archived = false; chatQuery = ''; chatLimit = 30; actionMessage = ''; actionError = ''; detailOpen = true;
  }
  function showUnreadable() { related = false; filterProjects = []; query = ''; period = 'all'; status = 'error'; detailOpen = false; }
  const recheck = (row: Worktree) => session.recheck(row);
  async function action(op: string, row: Worktree, extra = {}) {
    actionMessage = ''; actionError = '';
    try { await api(op, { scanId: snapshot?.scanId, worktreeId: row.id, ...extra }); actionMessage = '已向系统发送打开请求'; } catch(e) { actionError = String(e); }
  }
  function closePreview() { ++previewSequence; merge?.destroy(); merge = undefined; previewOpen = false; }
  async function preview(row: Worktree, change: Change, nextVersion = 'workspace') {
    previewRow = row; previewChange = change; version = nextVersion; previewError = ''; previewLoading = true;
    const seq = ++previewSequence; merge?.destroy(); merge = undefined;
    previewOpen = true;
    try { const text = await api<{ before: string; after: string }>('preview', { scanId: snapshot?.scanId, worktreeId: row.id, changeId: change.id, version });
      if(seq !== previewSequence) return; await tick();
      if(seq !== previewSequence || !previewOpen) return;
      merge = new MergeView({ parent: previewElement, a: { doc: text.before, extensions: [EditorView.editable.of(false), EditorState.readOnly.of(true)] }, b: { doc: text.after, extensions: [EditorView.editable.of(false), EditorState.readOnly.of(true)] }, collapseUnchanged: { margin: 3, minSize: 4 }, highlightChanges: true });
    } catch(e) { if(seq === previewSequence) previewError = String(e); } finally { if(seq === previewSequence) previewLoading = false; }
  }
  onMount(() => { void session.initialize(); void listen<typeof progress & { scanId: number }>('codex-workbench-progress', event => { if(loading) session.patch({ progress: event.payload }); }).then(fn => { if(alive) stop = fn; else fn(); }).catch(e => { if(alive) session.patch({ error: String(e) }); }); });
  onDestroy(() => { alive = false; Object.assign(retained, { query, filterProjects, related, period, date, status }); ++previewSequence; stop?.(); merge?.destroy(); });
</script>

<svelte:window on:keydown={(event) => {if (event.key === 'Escape' && moreOpen) {event.preventDefault(); moreOpen = false; document.querySelector<HTMLElement>('.more-filters summary')?.focus();}}}/>

<section class="workbench" aria-label="Codex工作台" tabindex="-1">
  <nav class="wb-tabs" aria-label="工作台视图"><button aria-pressed={view==='code'} on:click={()=>view='code'}>代码状态</button><button aria-pressed={view==='tasks'} on:click={()=>{detailOpen=false; view='tasks';}}>任务与验收</button></nav>
  {#if view==='tasks'}<TaskLedger workspaceHint={selected?.path??''}/>{:else}
  <div class="wb-body">
    <div class="scanbar">
      <div class="project-picker" on:focusout={(e) => { if (!(e.currentTarget.contains(e.relatedTarget as Node))) pickerOpen = false; }}>
        <div class="tags">{#each draft as id}<span class="tag">{projects.find(p => p.id === id)?.name}<button aria-label={`移除 ${projects.find(p => p.id === id)?.name}`} on:click={() => toggleProject(id)}><X size={12}/></button></span>{/each}<input bind:this={projectInput} aria-label="搜索并选择项目" placeholder={draft.length ? '' : '选择项目，键入搜索…'} bind:value={projectSearch} on:focus={() => pickerOpen = true} on:click={() => pickerOpen = true} on:input={() => pickerOpen = true} on:keydown={(e) => { if(e.key === 'Escape') {e.preventDefault(); pickerOpen = false;} if(e.key === 'ArrowDown') {e.preventDefault(); pickerOpen = true; tick().then(() => document.querySelector<HTMLButtonElement>('.project-options button')?.focus());} }} /></div>
        {#if pickerOpen}
          <div class="project-options" role="group" aria-label="项目选项">
            {#each projects.filter(p => !draft.includes(p.id) && p.name.toLowerCase().includes(projectSearch.toLowerCase())) as p}
              <button on:pointerdown={retainProjectSearchFocus} on:click={() => toggleProject(p.id)}>{p.name}<small>{p.roots.join(' · ')}</small></button>
            {:else}<p>没有其他匹配项目</p>{/each}
            {#if draft.length}<button on:pointerdown={retainProjectSearchFocus} on:click={() => updateProjectSelection([])}>清空选择</button>{/if}
          </div>
        {/if}
      </div>
      <button class="refresh" aria-label="刷新项目列表" disabled={catalogLoading} on:click={() => loadProjects()}><RefreshCw size={18}/></button>
      <button class="confirm" disabled={!draft.length || catalogLoading} on:click={scan} title={loading ? '重新开始扫描' : '扫描所选项目'}>确认</button>
    </div>
    <div class="scan-status" role="status">{#if loading}{progress.phase === 'history' ? '读取项目与会话索引…' : `检测工作树 ${progress.checked}/${progress.total}`}{:else if snapshot}{snapshot.projects.length} 个项目 · 扫描于 {fmt(snapshot.scannedAt)}{/if}{#if changed}<span>范围已变更，等待确认</span>{/if}</div>
    {#if error}<div class="error" role="alert">{error}<button aria-label="关闭错误" on:click={() => session.patch({ error: '' })}>×</button></div>{/if}
    {#if warnings.length}<div class="read-notice" role="status"><span>{#if unreadable.length}<strong>{unreadable.length} 棵工作树读取失败</strong> · {/if}{warnings.length} 条读取提示</span><div>{#if unreadable.length}<button on:click={showUnreadable}>仅看失败工作树</button>{/if}<button on:click={() => warningDialog.showModal()}>查看详情</button></div></div>{/if}
    <div class="overview" aria-label="工作树概览">
      <button class:chosen={status === 'all'} aria-pressed={status === 'all'} on:click={() => status = 'all'}>全部工作树 <strong>{base.length}</strong></button>
      <button class:chosen={status === 'dirty'} aria-pressed={status === 'dirty'} on:click={() => status = status === 'dirty' ? 'all' : 'dirty'}>未提交 <strong>{base.filter(r => statusMatches(r,'dirty')).length}</strong></button>
      <button class:chosen={status === 'unpushed'} aria-pressed={status === 'unpushed'} on:click={() => status = status === 'unpushed' ? 'all' : 'unpushed'}>待推送 <strong>{base.filter(r => statusMatches(r,'unpushed')).length}</strong></button>
      <span>{base.reduce((sum,r) => sum+r.conversations.length,0)} 条关联历史 · 含归档</span>
    </div>
    <div class="workspace">
      <section class="results" aria-label="工作树列表">
        <div class="filters">
          <label class="search"><Search size={14}/><input aria-label="搜索工作树与对话" bind:value={query} placeholder="搜分支、路径、对话标题"/></label>
          <details class="project-filter"><summary aria-label="筛选项目">{filterProjects.length ? `已选 ${filterProjects.length} 个项目` : '全部项目'}</summary><div>{#each snapshot?.projects ?? [] as p}<label class="check"><input type="checkbox" value={p.id} bind:group={filterProjects}/>{p.name}</label>{:else}<span>扫描后可筛选项目</span>{/each}</div></details>
          <select aria-label="最近活动" title="按关联主对话的最新更新时间计算，包含归档" bind:value={period}><option value="all">不限时间</option><option value="1">今天</option><option value="7">最近 7 天</option><option value="30">最近 30 天</option><option value="date">指定日期起</option></select>
          {#if period === 'date'}<input type="date" aria-label="起始日期" bind:value={date}/>{/if}
          <select aria-label="提交状态" bind:value={status}><option value="all">全部状态</option><option value="dirty">未提交</option><option value="unpushed">未推送</option><option value="pushed">无待推送提交</option><option value="clean">干净 · 无上游</option><option value="error">读取失败</option></select>
          <details class="more-filters" bind:open={moreOpen}
            on:focusout={(event) => {if (!event.currentTarget.contains(event.relatedTarget as Node)) moreOpen = false;}}><summary aria-label="更多筛选与说明" title="更多筛选与说明" class:active={related}><SlidersHorizontal size={16}/></summary><div>
            <label class="check"><input type="checkbox" bind:checked={related}/>仅有关联对话</label>
            <button on:click={() => {reset(); moreOpen = false;}}>重置筛选</button>
            <button on:click={() => {moreOpen = false; sourceDialog.showModal();}}>数据来源</button>
          </div></details>
        </div>
        <div class="column-head"><span>分支与工作目录</span><span>对话与活动</span><span>Git 状态</span></div>
        <div class="worktree-list">
          {#each rows as row (row.id)}<article class:selected={detailOpen && selected?.id === row.id} class="worktree-row">
            <button class="row-select" on:click={(event) => selectRow(row,event.currentTarget)} aria-expanded={detailOpen && selected?.id === row.id}>
              <span class="identity"><b class="branch">{branchLabel(row)}</b><span class="project-name">{row.projectName}{row.main ? ' · 主工作区' : ''}</span><small title={row.path}>{row.path.split(/[\\/]/).slice(-2).join('/')}</small></span>
              <span class="activity"><span>{row.conversations.length ? `${row.conversations.length} 条对话` : '未关联对话'}</span><span class="source-names">{[...new Set(row.conversations.map(c => c.source))].join(' · ')}</span><small>{fmt(activity(row))}</small></span>
              <span class="git-state" title={gitSummary(row)}>{#if row.git.error}<span class="badge danger">读取失败</span>{:else}{#if row.git.changes.some(c => c.conflict)}<span class="badge danger">有冲突</span>{/if}{#if dirty(row)}<span class="badge amber">未提交 {row.git.changes.length}</span>{/if}{#if unpushed(row)}<span class="badge blue">待推送 {row.git.ahead}</span>{/if}{#if !dirty(row) && !unpushed(row)}<span class="badge" class:green={row.git.upstream && !row.git.behind} class:amber={!!row.git.behind}>{row.git.upstream ? (row.git.behind ? '落后上游' : '已同步') : '干净 · 无上游'}</span>{/if}{/if}</span>
            </button>
            <button class="recheck" disabled={loading || rechecking.has(row.id)} on:click={() => recheck(row)}>{rechecking.has(row.id) ? '检测中…' : '重新检测'}</button>
          </article>{:else}<div class="empty">{loading ? '正在读取本地项目与对话…' : snapshot ? '没有符合条件的工作树' : '选择项目并确认，检查历史对话与 Git 状态。'}</div>{/each}
        </div>
      </section>

    </div>
  </div>
  {/if}
</section>
<AppSidePanel open={detailOpen} title={selected ? branchLabel(selected) : '工作树详情'}
  description="查看所选工作树的 Git 改动与关联 Codex 对话。" closeLabel="关闭工作树详情"
  returnFocus={selectedButton} onClose={() => detailOpen = false}>
      <aside class="detail">
        {#if selected}<div class="detail-eyebrow">{selected.projectName} · {selected.main ? '主工作区' : '工作树'}</div><p class="path">{selected.path}</p>
          <div class="actions"><button disabled={loading} on:click={() => action('openDirectory',selected)}><FolderOpen size={12}/>打开目录</button><button disabled={loading} on:click={() => action('openTerminal',selected)}><Terminal size={12}/>打开终端</button></div>
          <p class="git-summary" class:has-error={!!selected.git.error}>{gitSummary(selected)}</p>
          {#if selected.git.upstream}<p class="comparison-note">上游比较基于本地引用，未访问远端。</p>{/if}

          {#if selected.git.error}<p class="error">{selected.git.error}</p>{/if}
          <details class="changes" open={selected.git.changes.length > 0}><summary>文件改动 · {selected.git.changes.length} 项</summary><div class="change-list">{#each selected.git.changes as change}<div class="change"><button disabled={loading} aria-label={`预览 ${change.path}`} on:click={() => preview(selected,change)}><span class="file-kind" title={change.status}>{changeLabel(change)}</span><span>{change.original ? `${change.original} → ` : ''}{change.path}<small>{change.staged ? '含已暂存改动' : '工作区'}</small></span></button><button disabled={loading} aria-label={`定位 ${change.path}`} on:click={() => action('revealChange',selected,{changeId:change.id})}><FolderOpen size={13}/></button></div>{/each}</div></details>
          <div class="chat-heading"><strong>关联对话 <span>{filteredChats.length} 条</span></strong><label class="check"><input type="checkbox" bind:checked={archived}/>含归档</label></div>
          <p class="explanation">按会话工作目录关联，Git 改动属于整棵工作树。{#if archiveCount}{archiveCount} 条历史已归档。{/if}</p>
          {#if selected.conversations.length > 8}<label class="chat-search"><Search size={14}/><input aria-label="搜索关联对话" placeholder="搜索这棵工作树的对话" bind:value={chatQuery} on:input={() => chatLimit = 30}/></label>{/if}
          <div class="chats">{#each filteredChats.slice(0,chatLimit) as chat}<article><button disabled={loading} class="chat-link" on:click={() => action('openCodexChat',selected,{conversationId:chat.id})}>{chat.title} <ExternalLink size={12}/></button><small>{chat.source} · {fmt(chat.updatedAt)}{chat.archived ? ' · 已归档' : ''}</small></article>{:else}<p class="empty">{chatQuery ? '没有匹配的对话标题' : archiveCount && !archived ? '关联历史已归档，可开启“含归档”查看' : '没有找到此工作目录的主对话记录；仍可检查 Git 改动。'}</p>{/each}</div>
          {#if filteredChats.length > chatLimit}<button class="load-chats" on:click={() => chatLimit += 30}>再显示 {Math.min(30,filteredChats.length-chatLimit)} 条对话（还有 {filteredChats.length-chatLimit} 条）</button>{/if}
        {/if}
        {#if actionMessage}<p class="action-status" role="status">{actionMessage}</p>{/if}
        {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
      </aside>
</AppSidePanel>
<dialog bind:this={sourceDialog} class="wb-dialog source-dialog"><header><h3>数据来源与检查范围</h3><button aria-label="关闭数据来源" on:click={() => sourceDialog.close()}>×</button></header>
  <dl><dt>项目</dt><dd>读取 Codex 保存的项目与根目录。{sourceInfo?.projectCatalog === 'globalState' ? '当前使用本地工作目录列表作为后备来源。' : '优先使用 Codex 项目数据库。'}</dd>
  <dt>工作树与 Git 状态</dt><dd>从所选项目执行本地 Git 只读检查。未提交是文件条目数；未推送是相对本地上游引用的提交数，不执行 fetch。单棵重检只更新 Git 状态；新增工作树或对话需要重新扫描。</dd>
  <dt>关联对话</dt><dd>读取 Codex 的标题索引、会话数据库和后备会话头。按工作目录或其 Git 顶层关联；数量不是任务数或完成度。活动时间取关联主对话的最新更新时间，包含归档。</dd>
  <dt>本次数据目录</dt><dd>{#if sourceInfo}<span>Codex</span><code>{sourceInfo.codexHome}</code>{:else}读取项目后显示实际来源目录。{/if}</dd></dl>
  <p>Git 改动由同一工作树的对话共享，不自动归属到某条对话。本页不调用模型提取任务。</p>
</dialog>
<dialog bind:this={warningDialog} class="wb-dialog"><header><h3>读取失败详情</h3><button on:click={() => warningDialog.close()} aria-label="关闭读取失败详情">×</button></header>{#each warnings as warning}<p class="warning-text">{warning}</p>{/each}</dialog>
<Dialog.Root bind:open={previewOpen} onOpenChange={(open) => {if (!open) closePreview();}}>
  <Dialog.Content class="workbench-preview" showCloseButton={false}>
    <Dialog.Description class="sr-only">文件改动的只读对比，可切换工作区与暂存区。</Dialog.Description>
    <div class="preview-body"><header><Dialog.Title>{previewChange?.path}</Dialog.Title><button aria-label="关闭文件预览" on:click={closePreview}>×</button></header><div class="versions"><button class:active={version === 'workspace'} on:click={() => previewRow && previewChange && preview(previewRow,previewChange,'workspace')}>工作区</button><button class:active={version === 'index'} on:click={() => previewRow && previewChange && preview(previewRow,previewChange,'index')}>暂存区</button><span>HEAD ↔ {version === 'index' ? '暂存区' : '工作区'} · 只读</span></div>{#if previewLoading}<p role="status">读取文件…</p>{/if}{#if previewError}<p role="alert">{previewError}</p>{/if}<div class="merge" bind:this={previewElement}></div></div>
  </Dialog.Content>
</Dialog.Root>

<style>
  .workbench {
    background: var(--wb-bg);
    color: var(--wb-text);
    font: 13px/1.5 var(--font-family-ui);
    container-type: inline-size;
    min-width: 0;
    color-scheme: inherit;
  }
  .workbench *, .detail *, .preview-body * { box-sizing: border-box; }
  button, input, select { font: inherit; color: inherit; }
  button, input, select {
    border: 1px solid var(--wb-border);
    border-radius: 6px;
    background: var(--wb-panel);
  }
  button { cursor: pointer; padding: 6px 10px; }
  button:hover { border-color: var(--wb-border-strong); }
  button:disabled { opacity: .55; cursor: wait; }
  :is(button,input,select,summary):focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  input, select { min-width: 0; }
  .workbench:focus { outline: none; }
  .wb-tabs {
    display: flex;
    gap: 24px;
    width: 100%;
    margin: 0;
    padding-top: 0;
    border-bottom: 1px solid var(--wb-border);
  }
  .wb-tabs button { border: 0; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; padding: 12px 0 14px; color: var(--wb-muted); }
  .wb-tabs button[aria-pressed="true"] { border-bottom-color: var(--wb-text); color: var(--wb-text); font-weight: 600; }
  .wb-body { width: 100%; margin: 0; padding: 20px 0 0; }
  .scanbar { display: flex; gap: 8px; align-items: flex-start; }
  .project-picker { position: relative; flex: 1; min-width: 0; }
  .tags { min-height: 36px; display: flex; align-items: center; flex-wrap: wrap; gap: 5px; padding: 5px; background: var(--wb-panel); border: 1px solid var(--wb-border); border-radius: 7px; }
  .tags:focus-within { border-color: var(--wb-border-strong); }
  .workbench .tags input { flex: 1; width: 50px; min-width: 30px; min-height: 24px; height: 24px; padding: 2px 4px; border: 0; border-radius: 0; outline: none; background: transparent; font-size: 13px; line-height: 1.4; color: var(--wb-text); }
  .tag { display: flex; align-items: center; gap: 5px; background: var(--wb-control); border: 1px solid var(--wb-border); padding: 1px 5px; border-radius: 4px; max-width: 100%; overflow-wrap: anywhere; font-size: 12px; }
  .tag button { display: flex; padding: 3px; border: 0; background: transparent; color: var(--wb-muted); }
  .project-options { position: absolute; top: 100%; left: 0; right: 0; margin-top: 5px; z-index: 10; background: var(--wb-panel); border: 1px solid var(--wb-border-strong); border-radius: 7px; max-height: 290px; overflow: auto; padding: 5px; box-shadow: 0 10px 25px var(--wb-shadow); }
  .project-options button { display: block; width: 100%; text-align: left; background: transparent; border: 0; }
  .project-options button:hover { background: var(--wb-control); }
  .project-options small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--wb-muted); font-size: 11px; }
  .refresh { height: 36px; width: 36px; display: grid; place-items: center; flex: none; padding: 0; }
  .confirm { min-height: 36px; min-width: 72px; padding: 6px 18px; font-weight: 500; background: var(--wb-control); }
  .scan-status { display: flex; flex-wrap: wrap; gap: 12px; color: var(--wb-muted); font-size: 11px; }
  .scan-status:not(:empty) { margin-top: 7px; }
  .read-notice { display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-wrap: wrap; padding: 8px 10px; border: 1px solid var(--wb-border); border-radius: 6px; color: var(--wb-muted); margin-top: 12px; font-size: 12px; }
  .read-notice strong { color: var(--wb-text); font-weight: 500; }
  .read-notice>div { display: flex; gap: 8px; }
  .read-notice button { padding: 3px 7px; font-size: 12px; background: transparent; }
  .error { padding: 10px; color: var(--wb-danger-text); background: var(--wb-danger-bg); overflow-wrap: anywhere; border-radius: 5px; }
  .error>button { float: right; }
  .overview { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin: 14px 0 10px; }
  .overview button { display: flex; align-items: center; gap: 8px; min-height: 32px; padding: 5px 10px; border-color: transparent; background: transparent; font-size: 12px; color: var(--wb-muted); }
  .overview button.chosen { color: var(--wb-text); background: var(--wb-control); }
  .overview strong { font-size: 13px; font-weight: 600; font-variant-numeric: tabular-nums; color: var(--wb-text); }
  .overview>span { margin-left: auto; color: var(--wb-muted); font-size: 11px; }
  .workspace { border: 1px solid var(--wb-border); border-radius: 8px; background: var(--wb-panel); }
  .results { min-width: 0; }
  .filters { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 12px; border-bottom: 1px solid var(--wb-border); }
  .search { display: flex; align-items: center; gap: 5px; flex: 1; min-width: 160px; border: 1px solid var(--wb-border); border-radius: 6px; background: var(--wb-control); padding-left: 8px; color: var(--wb-muted); }
  .search:focus-within { border-color: var(--wb-border-strong); }
  .workbench .search input, .chat-search input { width: 100%; min-height: 32px; height: 32px; background: transparent; color: var(--wb-text); border: 0; padding: 4px 6px; font-size: 12px; line-height: 1.4; }
  .filters select, .filters>input { min-height: 34px; height: 34px; padding: 5px 8px; font-size: 12px; }
  .filters select { width: 120px; min-width: 96px; }
  .check { display: flex; align-items: center; white-space: nowrap; font-size: 11px; color: var(--wb-muted); gap: 5px; }
  .check input { width: 13px; height: 13px; margin: 0; accent-color: var(--wb-accent); }
  .project-filter, .more-filters { position: relative; flex: none; }
  summary { cursor: pointer; }
  .project-filter summary { padding: 7px 8px; border: 1px solid var(--wb-border); border-radius: 6px; white-space: nowrap; font-size: 12px; min-height: 34px; max-width: 150px; overflow: hidden; text-overflow: ellipsis; }
  .project-filter>div, .more-filters>div { position: absolute; top: calc(100% + 5px); z-index: 5; background: var(--wb-panel); padding: 12px; border: 1px solid var(--wb-border); border-radius: 6px; box-shadow: 0 6px 20px var(--wb-shadow); font-size: 12px; }
  .project-filter>div { left: 0; min-width: 200px; }
  .project-filter .check { padding: 6px 0; }
  .more-filters summary { display: grid; place-items: center; width: 34px; height: 34px; border: 1px solid transparent; border-radius: 6px; color: var(--wb-muted); list-style: none; }
  .more-filters summary::-webkit-details-marker { display: none; }
  .more-filters summary:hover, .more-filters summary.active { background: var(--wb-control); color: var(--wb-text); }
  .more-filters>div { right: 0; display: flex; flex-direction: column; align-items: stretch; gap: 8px; min-width: 168px; }
  .more-filters .check { padding: 6px 4px; }
  .more-filters button { text-align: left; border: 0; background: transparent; font-size: 12px; padding: 6px 4px; }
  .more-filters button:hover { background: var(--wb-control); }
  .column-head { display: grid; grid-template-columns: minmax(0,1.5fr) minmax(0,1fr) minmax(0,1fr) 80px; gap: 16px; padding: 9px 16px; color: var(--wb-muted); font-size: 11px; background: var(--wb-control); border-bottom: 1px solid var(--wb-border); }
  .worktree-list { padding: 0 6px 6px; }
  .worktree-row { position: relative; border-bottom: 1px solid var(--wb-border); background: transparent; padding-right: 86px; }
  .worktree-row:last-child { border-bottom: 0; }
  .worktree-row.selected { background: var(--wb-control); box-shadow: inset 2px 0 var(--wb-border-strong); border-radius: 4px; }
  .row-select { display: grid; grid-template-columns: minmax(0,1.5fr) minmax(0,1fr) minmax(0,1fr); gap: 16px; align-items: center; width: 100%; padding: 12px 10px; min-height: 88px; text-align: left; border: 0; background: transparent; font-size: 12px; border-radius: 4px; }
  .row-select:hover { background: var(--wb-sheen); }
  .identity { display: flex; align-items: flex-start; flex-direction: column; gap: 4px; min-width: 0; }
  .branch { font: 600 13px/1.4 var(--font-family-code); overflow-wrap: anywhere; }
  .project-name { font-size: 11px; color: var(--wb-muted); overflow-wrap: anywhere; }
  .identity>small { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--wb-muted); font-size: 11px; }
  .activity { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; min-width: 0; }
  .activity small, .source-names { color: var(--wb-muted); font-size: 11px; }
  .git-state { display: flex; align-items: flex-start; flex-wrap: wrap; gap: 5px 10px; }
  .badge { display: inline-block; color: var(--wb-neutral-text); font-size: 11px; font-weight: 400; line-height: 1.5; }
  .badge.amber { color: var(--wb-warning-text); }
  .badge.blue { color: var(--wb-info-text); }
  .badge.green { color: var(--wb-success-text); }
  .badge.danger { color: var(--wb-danger-text); }
  .recheck { position: absolute; top: 50%; right: 10px; transform: translateY(-50%); min-height: 28px; padding: 3px 8px; font-size: 11px; color: var(--wb-muted); background: transparent; }
  .detail { min-width: 0; }
  .detail-eyebrow { font-size: 12px; color: var(--wb-muted); margin-bottom: 5px; }
  .path { font: 11px/1.6 var(--font-family-code); overflow-wrap: anywhere; color: var(--wb-muted); margin: 0 0 12px; }
  .actions { display: flex; gap: 7px; flex-wrap: wrap; }
  .actions button { display: flex; align-items: center; gap: 5px; padding: 5px 8px; font-size: 12px; min-height: 30px; background: var(--wb-panel); }
  .git-summary { font-size: 12px; font-weight: 500; margin: 14px 0 4px; overflow-wrap: anywhere; }
  .git-summary.has-error { color: var(--wb-danger-text); }
  .comparison-note { font-size: 11px; color: var(--wb-muted); margin: 0 0 14px; }
  .changes { font-size: 12px; padding: 0 0 18px; border-bottom: 1px solid var(--wb-border); margin-top: 14px; }
  .changes summary { font-weight: 500; }
  .change-list { max-height: 220px; overflow: auto; margin-top: 8px; }
  .change { display: flex; align-items: center; gap: 4px; padding: 4px 0; }
  .change button { min-width: 0; padding: 6px; font-size: 12px; text-align: left; overflow-wrap: anywhere; min-height: 32px; background: var(--wb-control); border-color: transparent; }
  .change button:first-child { flex: 1; display: flex; align-items: baseline; gap: 8px; }
  .file-kind { flex: none; color: var(--wb-muted); font-size: 11px; }
  .change small { display: block; font-size: 10px; color: var(--wb-muted); margin-top: 2px; }
  .chat-heading { display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: 13px; margin-top: 18px; }
  .chat-heading strong { font-weight: 600; }
  .chat-heading strong span { font-size: 12px; font-weight: 400; color: var(--wb-muted); margin-left: 3px; }
  .explanation { color: var(--wb-muted); font-size: 11px; line-height: 1.7; margin: 8px 0 12px; }
  .chat-search { display: flex; align-items: center; gap: 4px; border: 1px solid var(--wb-border); border-radius: 6px; padding-left: 8px; color: var(--wb-muted); margin-bottom: 7px; }
  .chats article { padding: 11px 0; border-bottom: 1px solid var(--wb-border); }
  .chats article:last-child { border-bottom: 0; }
  .chat-link { font-size: 13px; color: var(--wb-text); text-align: left; padding: 0; border: 0; background: transparent; line-height: 1.6; overflow-wrap: anywhere; }
  .chat-link:hover { color: var(--wb-info-text); }
  .chat-link :global(svg) { display: inline; vertical-align: middle; color: var(--wb-muted); margin-left: 3px; }
  .chats small { display: block; font-size: 11px; color: var(--wb-muted); margin-top: 4px; }
  .load-chats { width: 100%; font-size: 12px; min-height: 34px; margin-top: 10px; background: var(--wb-control); }
  .empty { text-align: center; padding: 40px 16px; color: var(--wb-muted); font-size: 12px; }
  .action-status { font-size: 12px; color: var(--wb-muted); margin: 12px 0 0; }
  .wb-dialog { font: 13px/1.6 var(--font-family-ui); color: var(--wb-text); background: var(--wb-panel); border: 1px solid var(--wb-border); border-radius: 10px; max-width: min(90vw,700px); max-height: 85vh; padding: 20px; color-scheme: inherit; }
  .wb-dialog::backdrop { background: var(--wb-scrim); backdrop-filter: blur(3px); }
  .wb-dialog header, .preview-body header { display: flex; align-items: center; justify-content: space-between; gap: 15px; margin-bottom: 14px; }
  .wb-dialog h3, .preview-body :global(h3) { margin: 0; font-size: 15px; overflow-wrap: anywhere; }
  .wb-dialog header button, .preview-body header button { min-width: 30px; min-height: 30px; }
  .warning-text { font-size: 12px; overflow-wrap: anywhere; border-bottom: 1px solid var(--wb-border); padding-bottom: 8px; }
  .source-dialog dl { margin: 0; }
  .source-dialog dt { font-size: 13px; font-weight: 600; margin-top: 14px; }
  .source-dialog dd { margin: 5px 0 0; font-size: 12px; color: var(--wb-muted); }
  .source-dialog code { display: block; font-size: 11px; overflow-wrap: anywhere; margin: 4px 0 10px; color: var(--wb-text); }
  .source-dialog>p { font-size: 12px; color: var(--wb-muted); border-top: 1px solid var(--wb-border); padding-top: 12px; }
  :global(.workbench-preview) { width: min(1300px,94vw); max-width: 94vw; border: 1px solid var(--wb-border); border-radius: 10px; background: var(--wb-panel); color: var(--wb-text); font: 13px/1.6 var(--font-family-ui); color-scheme: inherit; }
  .preview-body { padding: 20px; min-width: 0; }
  .versions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin: 15px 0; font-size: 12px; }
  .versions span { margin-left: auto; color: var(--wb-muted); }
  .versions button.active { border-color: var(--wb-accent); }
  .merge { max-height: 65vh; overflow: auto; color: var(--wb-text); }
  .merge :global(.cm-editor) { background: var(--wb-bg); color: var(--wb-text); font-size: 12px; }
  .merge :global(.cm-gutters) { background: var(--wb-control); color: var(--wb-muted); }
  .merge :global(.cm-mergeView) { max-height: 65vh; overflow: auto; }
  .merge :global(.cm-merge-a .cm-changedLine), .merge :global(.cm-deletedLine) { background: var(--wb-danger-bg); }
  .merge :global(.cm-merge-b .cm-changedLine), .merge :global(.cm-insertedLine) { background: var(--wb-success-bg); }
  .merge :global(.cm-merge-a .cm-changedText), .merge :global(.cm-deletedText) { background: var(--wb-deleted-text); }
  .merge :global(.cm-merge-b .cm-changedText), .merge :global(.cm-insertedText) { background: var(--wb-inserted-text); }
  .merge :global(.cm-merge-a .cm-changedLineGutter) { background: var(--wb-danger-text); }
  .merge :global(.cm-merge-b .cm-changedLineGutter) { background: var(--wb-success-text); }
  .merge :global(.cm-collapsedLines) { background: var(--wb-control); color: var(--wb-muted); }
  @container(max-width:760px) {
    .filters { flex-wrap: wrap; }
    .search { flex-basis: 100%; }
    .filters select { flex: 1; width: 110px; }
    .more-filters { margin-left: auto; }
    .column-head { grid-template-columns: minmax(0,1.5fr) minmax(0,1fr) minmax(0,.8fr); gap: 10px; padding-right: 16px; }
    .worktree-row { padding-right: 0; }
    .row-select { grid-template-columns: minmax(0,1.5fr) minmax(0,1fr) minmax(0,.8fr); gap: 10px; padding-bottom: 38px; }
    .recheck { top: auto; bottom: 8px; transform: none; }
  }
  @container(max-width:460px) {
    .wb-body { padding-top: 16px; }
    .confirm { min-width: 60px; padding: 6px 12px; }
    .overview { gap: 2px; }
    .overview button { padding: 5px 8px; gap: 5px; }
    .overview>span { flex-basis: 100%; margin: 2px 0 0 8px; }
    .filters { gap: 6px; padding: 10px; }
    .filters select { width: 90px; padding: 5px; }
    .project-filter summary { max-width: 105px; padding: 7px 5px; }
    .branch { font-size: 12px; }
    .git-state { flex-direction: column; }
  }
  @media(max-width:520px) { .preview-body { padding: 16px; } }
</style>
