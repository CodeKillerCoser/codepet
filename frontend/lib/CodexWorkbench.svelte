<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { Search, RefreshCw, FolderOpen, Terminal, ExternalLink, X } from '@lucide/svelte';
  import { MergeView } from '@codemirror/merge';
  import { EditorView } from '@codemirror/view';
  import { EditorState } from '@codemirror/state';
  import { workbenchRequest as api, activity, dirty, unpushed, filterRows, statusMatches, branchLabel, changeLabel, gitSummary, type SourceInfo, type Project, type Snapshot, type Worktree, type Change } from './codexWorkbench';
  let projects: Project[] = [], draft: string[] = [], projectSearch = '', pickerOpen = false;
  let snapshot: Snapshot | null = null, loading = false, catalogLoading = true, error = '', catalogWarnings: string[] = [];
  let query = '', filterProjects: string[] = [], related = false, period = 'all', date = '', status = 'all', selectedId = '', archived = false;
  let progress = { checked: 0, total: 0, phase: 'history' }, sequence = 0, rechecking = new Set<string>(), actionMessage = '';
  let warningDialog: HTMLDialogElement, previewDialog: HTMLDialogElement, previewElement: HTMLDivElement, previewChange: Change | null = null, previewRow: Worktree | null = null, version = 'workspace', previewError = '', previewLoading = false, previewSequence = 0;
  let sourceInfo: SourceInfo | undefined, sourceDialog: HTMLDialogElement;
  let mobileDetail = false, detailHeading: HTMLHeadingElement, selectedButton: HTMLButtonElement | undefined;
  let chatQuery = '', chatLimit = 30;
  let alive = true;
  let merge: MergeView | undefined, stop: (() => void) | undefined;
  $: base = filterRows(snapshot?.worktrees ?? [], { query, projects: filterProjects, related, period, date });
  $: rows = base.filter(r => statusMatches(r, status));
  $: selected = rows.find(r => r.id === selectedId) ?? rows[0];
  $: chats = selected?.conversations.filter(c => archived || !c.archived).sort((a,b) => b.updatedAt-a.updatedAt) ?? [];
  $: filteredChats = chats.filter(c => !chatQuery.trim() || c.title.toLocaleLowerCase().includes(chatQuery.trim().toLocaleLowerCase()));
  $: unreadable = snapshot?.worktrees.filter(r => r.git.error) ?? [];
  $: warnings = [...new Set([...catalogWarnings, ...(snapshot?.warnings ?? []), ...unreadable.map(r => `${r.path}：${r.git.error}`)])];
  $: archiveCount = selected?.conversations.filter(c => c.archived).length ?? 0;
  $: changed = !!snapshot && [...draft].sort().join() !== snapshot.projects.map(p => p.id).sort().join();
  const fmt = (n: number) => n ? new Date(n).toLocaleString('zh-CN', { month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false }) : '暂无活动';
  async function loadProjects(initial = false) {
    catalogLoading = true; error = '';
    try { const result = await api<{ projects: Project[]; warnings: string[]; sourceInfo?: SourceInfo }>('projects'); projects = result.projects; catalogWarnings = result.warnings; sourceInfo = result.sourceInfo;
      if (initial) draft = (await api<string[]>('preferences')).filter(id => projects.some(p => p.id === id));
      else { draft = draft.filter(id => projects.some(p => p.id === id)); await persist(); }
    } catch(e) { error = String(e); } finally { catalogLoading = false; }
  }
  let saveQueue = Promise.resolve();
  function persist() { const projectIds = [...draft]; saveQueue = saveQueue.then(async () => { await api('saveSelection', { projectIds }); }).catch(e => { error = String(e); }); return saveQueue; }
  function toggleProject(id: string) { draft = draft.includes(id) ? draft.filter(p => p !== id) : [...draft, id]; projectSearch = ''; void persist(); }
  async function scan() {
    if (previewDialog?.open) closePreview();
    const seq = ++sequence; loading = true; error = ''; progress = { checked: 0, total: 0, phase: 'history' }; pickerOpen = false;
    try { const result = await api<Snapshot>('scan', { projectIds: [...draft] }); if(seq !== sequence) return; snapshot = result; sourceInfo = result.sourceInfo ?? sourceInfo; selectedId = ''; mobileDetail = false; chatQuery = ''; chatLimit = 30; filterProjects = filterProjects.filter(id => result.projects.some(p => p.id === id)); }
    catch(e) { if(seq === sequence) { error = String(e); snapshot = null; } } finally { if(seq === sequence) loading = false; }
  }
  function reset() { query = ''; filterProjects = []; related = false; period = 'all'; date = ''; status = 'all'; }
  async function selectRow(row: Worktree, button: HTMLButtonElement) {
    selectedId = row.id; selectedButton = button; archived = false; chatQuery = ''; chatLimit = 30; mobileDetail = true;
    await tick(); detailHeading?.focus({ preventScroll: true });
  }
  async function backToList() { mobileDetail = false; await tick(); selectedButton?.focus({ preventScroll: true }); }
  function showUnreadable() { related = false; filterProjects = []; query = ''; period = 'all'; status = 'error'; mobileDetail = false; }
  async function recheck(row: Worktree) {
    if (!snapshot || rechecking.has(row.id)) return; const scanId = snapshot.scanId; rechecking = new Set([...rechecking, row.id]);
    try { const updated = await api<Worktree>('recheck', { scanId, worktreeId: row.id }); if (snapshot?.scanId === scanId) snapshot = { ...snapshot, worktrees: snapshot.worktrees.map(r => r.id === row.id ? updated : r) }; }
    catch(e) { error = String(e); } finally { rechecking = new Set([...rechecking].filter(id => id !== row.id)); }
  }
  async function action(op: string, row: Worktree, extra = {}) {
    actionMessage = '';
    try { await api(op, { scanId: snapshot?.scanId, worktreeId: row.id, ...extra }); actionMessage = '已向系统发送打开请求'; } catch(e) { error = String(e); }
  }
  function closePreview() { ++previewSequence; merge?.destroy(); merge = undefined; previewDialog.close(); }
  async function preview(row: Worktree, change: Change, nextVersion = 'workspace') {
    previewRow = row; previewChange = change; version = nextVersion; previewError = ''; previewLoading = true;
    const seq = ++previewSequence; merge?.destroy(); merge = undefined;
    if (!previewDialog.open) previewDialog.showModal();
    try { const text = await api<{ before: string; after: string }>('preview', { scanId: snapshot?.scanId, worktreeId: row.id, changeId: change.id, version });
      if(seq !== previewSequence) return; await tick();
      merge = new MergeView({ parent: previewElement, a: { doc: text.before, extensions: [EditorView.editable.of(false), EditorState.readOnly.of(true)] }, b: { doc: text.after, extensions: [EditorView.editable.of(false), EditorState.readOnly.of(true)] }, collapseUnchanged: { margin: 3, minSize: 4 }, highlightChanges: true });
    } catch(e) { if(seq === previewSequence) previewError = String(e); } finally { if(seq === previewSequence) previewLoading = false; }
  }
  onMount(() => { void loadProjects(true); void listen<typeof progress & { scanId: number }>('codex-workbench-progress', event => { if(loading) progress = event.payload; }).then(fn => { if(alive) stop = fn; else fn(); }).catch(e => { if(alive) error = String(e); }); });
  onDestroy(() => { alive = false; if(loading) void api('cancel').catch(() => {}); ++sequence; ++previewSequence; stop?.(); merge?.destroy(); });
</script>

<section class="workbench" class:detail-open={mobileDetail} aria-label="Agent工作台" tabindex="-1">

  <div class="wb-body">
    <div class="scope-heading"><div class="scope-caption"><strong>检查范围</strong><span>从 Codex 项目目录选择；确认后扫描 Git 与本地对话</span></div><div class="scope-tools"><span class="local"><i></i>本机 · 只读</span><button class="source-help" on:click={() => sourceDialog.showModal()}>数据来源</button></div></div><div class="scanbar">
      <div class="project-picker" on:focusout={(e) => { if (!(e.currentTarget.contains(e.relatedTarget as Node))) pickerOpen = false; }}>
        <div class="tags">{#each draft as id}<span class="tag">{projects.find(p => p.id === id)?.name}<button aria-label={`移除 ${projects.find(p => p.id === id)?.name}`} on:click={() => toggleProject(id)}><X size={12}/></button></span>{/each}<input aria-label="搜索并选择项目" placeholder={draft.length ? '' : '选择项目，键入搜索…'} bind:value={projectSearch} on:focus={() => pickerOpen = true} on:input={() => pickerOpen = true} on:keydown={(e) => { if(e.key === 'Escape') pickerOpen = false; if(e.key === 'ArrowDown') {e.preventDefault(); pickerOpen = true; tick().then(() => document.querySelector<HTMLButtonElement>('.project-options button')?.focus());} }} /></div>
        {#if pickerOpen}<div class="project-options" role="group" aria-label="项目选项">{#each projects.filter(p => !draft.includes(p.id) && p.name.toLowerCase().includes(projectSearch.toLowerCase())) as p}<button on:click={() => toggleProject(p.id)}>{p.name}<small>{p.roots.join(' · ')}</small></button>{:else}<p>没有其他匹配项目</p>{/each}{#if draft.length}<button on:click={() => { draft = []; void persist(); }}>清空选择</button>{/if}</div>{/if}
      </div>
      <button class="refresh" aria-label="刷新项目列表" disabled={catalogLoading} on:click={() => loadProjects()}><RefreshCw size={18}/></button>
      <div class="scan-action"><button class="primary" disabled={!draft.length || catalogLoading} on:click={scan}>{loading ? '重新开始扫描' : snapshot && !changed ? '重新扫描' : '确认并扫描'}</button><small aria-live="polite">{loading ? (progress.phase === 'history' ? '读取项目与会话索引…' : `检测工作树 ${progress.checked}/${progress.total}`) : snapshot ? `${snapshot.projects.length} 个项目 · 扫描于 ${fmt(snapshot.scannedAt)}` : '选择项目后确认扫描'}{#if changed}<br/>项目选择已变更，等待确认{/if}</small></div>
    </div>
    {#if error}<div class="error" role="alert">{error}<button aria-label="关闭错误" on:click={() => error = ''}>×</button></div>{/if}
    {#if warnings.length}<div class="read-notice" role="status"><span>{#if unreadable.length}<strong>{unreadable.length} 棵工作树读取失败</strong> · {/if}{warnings.length} 条读取提示</span><div>{#if unreadable.length}<button on:click={showUnreadable}>仅看失败工作树</button>{/if}<button on:click={() => warningDialog.showModal()}>查看详情</button></div></div>{/if}
    <div class="overview">
      <button class:chosen={status === 'dirty'} on:click={() => status = status === 'dirty' ? 'all' : 'dirty'}><span><i class="amber"></i>未提交工作树</span><strong>{base.filter(r => statusMatches(r,'dirty')).length}</strong><small>{base.filter(r => !r.git.error).reduce((n,r) => n+r.git.changes.length,0)} 项文件改动待检查</small></button>
      <button class:chosen={status === 'unpushed'} on:click={() => status = status === 'unpushed' ? 'all' : 'unpushed'}><span><i class="blue"></i>待推送工作树</span><strong>{base.filter(r => statusMatches(r,'unpushed')).length}</strong><small>按本地上游引用比较</small></button>
      <button class:chosen={status === 'all'} on:click={() => status = 'all'}><span><i></i>全部工作树</span><strong>{base.length}</strong><small>{base.reduce((sum,r) => sum+r.conversations.length,0)} 条关联历史 · 含归档</small></button>
    </div>
    <div class="workspace" class:viewing-detail={mobileDetail}>
      <section class="results" aria-label="工作树列表">
        <div class="results-heading"><h3>工作树</h3><span>优先显示冲突、未提交和未推送</span></div><div class="filters">
          <div class="searchline"><label class="search"><Search size={14}/><input aria-label="搜索工作树与对话" bind:value={query} placeholder="搜分支、路径、对话标题"/></label><label class="check"><input type="checkbox" bind:checked={related}/>仅有关联对话</label><button class="plain" on:click={reset}>重置</button><span>{rows.length} 棵工作树</span></div>
          <div class="filterline"><div class="project-filter"><span>项目</span><details><summary>{filterProjects.length ? `已选 ${filterProjects.length} 个项目` : '全部项目'}</summary><div>{#each snapshot?.projects ?? [] as p}<label class="check"><input type="checkbox" value={p.id} bind:group={filterProjects}/>{p.name}</label>{/each}</div></details></div><label title="按关联主对话的最新更新时间计算，包含归档">对话活动<select aria-label="最近活动" bind:value={period}><option value="all">不限时间</option><option value="1">今天</option><option value="7">最近 7 天</option><option value="30">最近 30 天</option><option value="date">指定日期起</option></select>{#if period === 'date'}<input type="date" aria-label="起始日期" bind:value={date}/>{/if}</label><label>提交状态<select aria-label="提交状态" bind:value={status}><option value="all">全部状态</option><option value="dirty">未提交</option><option value="unpushed">未推送</option><option value="pushed">无待推送提交</option><option value="clean">干净 · 无上游</option><option value="error">读取失败</option></select></label></div>
        </div>
        <div class="column-head"><span>分支与工作目录</span><span>对话与活动</span><span>Git 状态</span></div>
        <div class="worktree-list">
          {#each rows as row (row.id)}<article class:selected={selected?.id === row.id} class="worktree-row">
            <button class="row-select" on:click={(event) => selectRow(row,event.currentTarget)} aria-pressed={selected?.id === row.id}>
              <span class="identity"><b class="branch">{branchLabel(row)}</b><span class="project-name">{row.projectName}{row.main ? ' · 主工作区' : ''}</span><small title={row.path}>{row.path.split(/[\\/]/).slice(-2).join('/')}</small></span>
              <span class="activity"><span>{row.conversations.length ? `${row.conversations.length} 条对话` : '未关联对话'}</span><span class="source-names">{[...new Set(row.conversations.map(c => c.source))].join(' · ')}</span><small>{fmt(activity(row))}</small></span>
              <span class="git-state" title={gitSummary(row)}>{#if row.git.error}<span class="badge danger">读取失败</span>{:else}{#if row.git.changes.some(c => c.conflict)}<span class="badge danger">有冲突</span>{/if}{#if dirty(row)}<span class="badge amber">未提交 {row.git.changes.length}</span>{/if}{#if unpushed(row)}<span class="badge blue">待推送 {row.git.ahead}</span>{/if}{#if !dirty(row) && !unpushed(row)}<span class="badge" class:green={row.git.upstream && !row.git.behind} class:amber={!!row.git.behind}>{row.git.upstream ? (row.git.behind ? '落后上游' : '已同步') : '干净 · 无上游'}</span>{/if}{/if}</span>
            </button>
            <button class="recheck" disabled={loading || rechecking.has(row.id)} on:click={() => recheck(row)}>{rechecking.has(row.id) ? '检测中…' : '重新检测'}</button>
          </article>{:else}<div class="empty">{loading ? '正在读取本地项目与对话…' : snapshot ? '没有符合条件的工作树' : '选择顶部项目，点击“确认并扫描”检查历史对话与 Git 状态。'}</div>{/each}
        </div>
      </section>
      <aside class="detail" aria-label="工作树详情"><button class="back-to-list" on:click={backToList}>← 返回工作树列表</button>
        {#if selected}<div class="detail-eyebrow">{selected.projectName} · {selected.main ? '主工作区' : '工作树'}</div><h3 bind:this={detailHeading} tabindex="-1">{branchLabel(selected)}</h3><p class="path">{selected.path}</p>
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
        {:else}<div class="empty">选择工作树查看关联对话</div>{/if}
        {#if actionMessage}<p class="action-status" role="status">{actionMessage}</p>{/if}
      </aside>
    </div>
  </div>
</section>
<dialog bind:this={sourceDialog} class="wb-dialog source-dialog"><header><h3>数据来源与检查范围</h3><button aria-label="关闭数据来源" on:click={() => sourceDialog.close()}>×</button></header>
  <dl><dt>项目</dt><dd>读取 Codex 保存的项目与根目录。{sourceInfo?.projectCatalog === 'globalState' ? '当前使用本地工作目录列表作为后备来源。' : '优先使用 Codex 项目数据库。'}</dd>
  <dt>工作树与 Git 状态</dt><dd>从所选项目执行本地 Git 只读检查。未提交是文件条目数；未推送是相对本地上游引用的提交数，不执行 fetch。单棵重检只更新 Git 状态；新增工作树或对话需要重新扫描。</dd>
  <dt>关联对话</dt><dd>读取 Codex 的标题索引、会话数据库和后备会话头。按工作目录或其 Git 顶层关联；数量不是任务数或完成度。活动时间取关联主对话的最新更新时间，包含归档。</dd>
  <dt>本次数据目录</dt><dd>{#if sourceInfo}<span>Codex</span><code>{sourceInfo.codexHome}</code>{:else}读取项目后显示实际来源目录。{/if}</dd></dl>
  <p>Git 改动由同一工作树的对话共享，不自动归属到某条对话。本页不调用模型提取任务。</p>
</dialog>
<dialog bind:this={warningDialog} class="wb-dialog"><header><h3>读取失败详情</h3><button on:click={() => warningDialog.close()} aria-label="关闭读取失败详情">×</button></header>{#each warnings as warning}<p class="warning-text">{warning}</p>{/each}</dialog>
<dialog bind:this={previewDialog} class="wb-dialog preview" on:cancel={closePreview} on:close={() => { ++previewSequence; merge?.destroy(); merge = undefined; }}><header><h3>{previewChange?.path}</h3><button aria-label="关闭文件预览" on:click={closePreview}>×</button></header><div class="versions"><button class:active={version === 'workspace'} on:click={() => previewRow && previewChange && preview(previewRow,previewChange,'workspace')}>工作区</button><button class:active={version === 'index'} on:click={() => previewRow && previewChange && preview(previewRow,previewChange,'index')}>暂存区</button><span>HEAD ↔ {version === 'index' ? '暂存区' : '工作区'} · 只读</span></div>{#if previewLoading}<p role="status">读取文件…</p>{/if}{#if previewError}<p role="alert">{previewError}</p>{/if}<div class="merge" bind:this={previewElement}></div></dialog>

<style>
  .workbench{background:var(--wb-bg);color:var(--wb-text);font:13px/1.5 var(--font-family-ui);container-type:inline-size;min-width:0;color-scheme:inherit}
  .workbench *{box-sizing:border-box}button,input,select{font:inherit;color:inherit}button{cursor:pointer}button:disabled{opacity:.55;cursor:wait}
  button,input,select{border:1px solid var(--wb-border);border-radius:6px;background:var(--wb-panel)}button{padding:6px 10px}button:hover{border-color:var(--wb-border-strong)}
  :is(button,input,select,summary):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}input,select{min-width:0}
  .workbench:focus{outline:none}.local{font-size:11px;color:var(--wb-muted);display:flex;align-items:center;gap:6px;white-space:nowrap}.source-help{font-size:12px;color:var(--wb-muted);background:transparent;border-color:transparent}

  i{display:inline-block;width:6px;height:6px;border-radius:50%;background:var(--wb-success);flex:none}i.amber{background:var(--wb-warning)}i.blue{background:var(--wb-info-text)}
  .wb-body{width:min(1400px,92%);margin:auto;padding:24px 0 28px}.scope-heading{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:10px}.scope-caption{display:flex;align-items:baseline;gap:12px;min-width:0}.scope-tools{display:flex;align-items:center;gap:10px;flex:none}.scope-heading strong{font-size:13px}.scope-heading span{font-size:12px;color:var(--wb-muted)}
  .scanbar{display:flex;gap:8px;align-items:flex-start;margin-bottom:20px}.project-picker{position:relative;flex:1;min-width:0}.tags{min-height:40px;display:flex;align-items:center;flex-wrap:wrap;gap:5px;padding:5px;background:var(--wb-panel);border:1px solid var(--wb-border);border-radius:7px}.tags:focus-within{border-color:var(--wb-accent)}
  .workbench .tags input{flex:1;width:50px;min-width:30px;min-height:22px;height:22px;padding:2px;border:0;border-radius:0;outline:none;background:transparent;font-size:13px;line-height:1.4;color:var(--wb-text)}
  .tag{display:flex;align-items:center;gap:5px;background:var(--wb-control);border:1px solid var(--wb-border);padding:1px 5px;border-radius:4px;max-width:100%;overflow-wrap:anywhere;font-size:12px}.tag button{display:flex;padding:3px;border:0;background:transparent;color:var(--wb-muted)}
  .project-options{position:absolute;top:100%;left:0;right:0;margin-top:5px;z-index:10;background:var(--wb-panel);border:1px solid var(--wb-border-strong);border-radius:7px;max-height:290px;overflow:auto;padding:5px;box-shadow:0 10px 25px var(--wb-shadow)}.project-options button{display:block;width:100%;text-align:left;background:transparent;border:0}.project-options button:hover{background:var(--wb-control)}.project-options small{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--wb-muted);font-size:11px}
  .refresh{height:40px;width:40px;display:grid;place-items:center;flex:none}.scan-action{margin-left:6px;width:185px;display:flex;flex-direction:column;align-items:flex-end;gap:6px;flex:none}.primary{min-height:40px;background:var(--wb-primary);color:var(--wb-on-primary);font-weight:600;border-color:var(--wb-primary);padding:8px 16px}.scan-action small{font-size:11px;color:var(--wb-muted);align-self:stretch;line-height:1.5;text-align:right}
  .read-notice{display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap;padding:10px 12px;border:1px solid var(--wb-border);border-left:3px solid var(--wb-warning);border-radius:6px;background:var(--wb-panel);color:var(--wb-muted);margin-bottom:16px;font-size:12px}.read-notice strong{color:var(--wb-text);font-weight:500}.read-notice>div{display:flex;gap:8px}.read-notice button{padding:3px 7px;font-size:12px;background:transparent}
  .error{padding:10px;color:var(--wb-danger-text);background:var(--wb-danger-bg);overflow-wrap:anywhere;border-radius:5px}.error>button{float:right}
  .overview{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px;margin-bottom:20px}.overview button{position:relative;display:flex;flex-direction:column;align-items:flex-start;gap:2px;padding:13px 15px;border-radius:8px;background:var(--wb-panel);text-align:left}.overview button.chosen{border-color:var(--wb-border-strong);box-shadow:inset 0 -2px var(--wb-accent)}.overview span{display:flex;align-items:center;gap:7px;font-size:12px;font-weight:500;color:var(--wb-muted)}.overview strong{font-size:28px;line-height:1.35;font-weight:600;font-variant-numeric:tabular-nums}.overview small{font-size:11px;color:var(--wb-muted)}
  .workspace{display:grid;grid-template-columns:minmax(0,1.5fr) minmax(0,1fr);border:1px solid var(--wb-border);border-radius:10px;background:var(--wb-panel);overflow:hidden;min-height:540px;align-items:start}.results{min-width:0;border-right:1px solid var(--wb-border)}
  .results-heading{display:flex;align-items:baseline;justify-content:space-between;gap:12px;padding:16px 16px 0}.results-heading h3{font-size:14px;margin:0;font-weight:600}.results-heading>span{font-size:11px;color:var(--wb-muted)}
  .filters{padding:12px 16px 14px;border-bottom:1px solid var(--wb-border)}.searchline{display:flex;align-items:center;gap:9px;flex-wrap:wrap;font-size:12px;color:var(--wb-muted)}.search{display:flex;align-items:center;gap:5px;flex:1;min-width:170px;border:1px solid var(--wb-border);border-radius:6px;background:var(--wb-control);padding-left:8px}.search:focus-within{border-color:var(--wb-accent)}
  .workbench .search input,.workbench .chat-search input{width:100%;min-height:30px;height:30px;background:transparent;color:var(--wb-text);border:0;padding:4px 6px;font-size:12px;line-height:1.4}
  .check{display:flex;align-items:center;white-space:nowrap;font-size:11px}.workbench .check{min-height:0;color:var(--wb-muted);gap:5px}.workbench .check input{width:13px;height:13px;margin:0;accent-color:var(--wb-accent)}.plain{border:0;padding:3px 0;background:transparent;color:var(--wb-muted);font-size:12px}
  .filterline{display:grid;grid-template-columns:1.1fr 1fr 1fr;gap:10px;margin-top:12px}.filterline label,.project-filter{display:flex;flex-direction:column;gap:5px;font-size:12px;color:var(--wb-muted);min-width:0}.workbench .filterline select{width:100%;min-height:34px;height:34px;background:var(--wb-panel);color:var(--wb-text);border:1px solid var(--wb-border);padding:6px 8px;font-size:12px;line-height:1.4}
  .project-filter{position:relative}.project-filter summary{padding:7px 8px;background:var(--wb-panel);border:1px solid var(--wb-border);border-radius:6px;white-space:nowrap;min-height:34px}.project-filter details>div{position:absolute;top:100%;left:0;min-width:200px;max-width:100%;z-index:5;background:var(--wb-panel);padding:12px;border:1px solid var(--wb-border);border-radius:6px}.project-filter details>div label{flex-direction:row;margin-bottom:8px;justify-content:flex-start}
  .column-head{display:grid;grid-template-columns:1.4fr 1fr .8fr;gap:12px;padding:9px 16px;color:var(--wb-muted);font-size:11px;background:var(--wb-control);border-bottom:1px solid var(--wb-border)}.worktree-list{max-height:680px;overflow:auto;padding:0 6px 6px}
  .worktree-row{position:relative;border-bottom:1px solid var(--wb-border);background:transparent}.worktree-row:last-child{border-bottom:0}.worktree-row.selected{background:var(--wb-control);box-shadow:inset 3px 0 var(--wb-accent);border-radius:4px}.row-select{display:grid;grid-template-columns:1.4fr 1fr .8fr;gap:12px;align-items:center;width:100%;padding:14px 10px 40px;min-height:116px;text-align:left;border:0;background:transparent;font-size:12px;border-radius:4px}.row-select:hover{background:var(--wb-sheen)}
  .identity{display:flex;align-items:flex-start;flex-direction:column;gap:5px;min-width:0}.branch{font:600 13px/1.4 var(--font-family-code);overflow-wrap:anywhere}.project-name{font-size:11px;color:var(--wb-muted);overflow-wrap:anywhere}.identity>small{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--wb-muted);font-size:11px}
  .activity{display:flex;flex-direction:column;align-items:flex-start;gap:4px;min-width:0}.activity small,.source-names{color:var(--wb-muted);font-size:11px}.git-state{display:flex;align-items:flex-start;flex-direction:column;gap:5px}.badge{display:inline-block;color:var(--wb-neutral-text);font-size:11px;font-weight:400;line-height:1.5}.badge.amber{color:var(--wb-warning-text)}.badge.blue{color:var(--wb-info-text)}.badge.green{color:var(--wb-success-text)}.badge.danger{color:var(--wb-danger-text)}
  .recheck{position:absolute;bottom:8px;right:10px;min-height:28px;padding:3px 8px;font-size:11px;color:var(--wb-muted);background:var(--wb-panel)}
  .detail{position:relative;min-width:0;max-height:900px;overflow:auto;padding:20px}.detail-eyebrow{font-size:12px;color:var(--wb-muted);margin-bottom:5px}.detail h3{font-size:16px;line-height:1.4;margin:0 0 9px;overflow-wrap:anywhere;font-weight:600}.detail h3:focus{outline:none}.path{font:11px/1.6 var(--font-family-code);overflow-wrap:anywhere;color:var(--wb-muted);margin:0 0 12px}.actions{display:flex;gap:7px;flex-wrap:wrap}.actions button{display:flex;align-items:center;gap:5px;padding:5px 8px;font-size:12px;min-height:30px;background:var(--wb-panel)}
  .git-summary{font-size:12px;font-weight:500;margin:14px 0 4px;overflow-wrap:anywhere}.git-summary.has-error{color:var(--wb-danger-text)}.comparison-note{font-size:11px;color:var(--wb-muted);margin:0 0 14px}.changes{font-size:12px;padding:0 0 18px;border-bottom:1px solid var(--wb-border);margin-top:14px}summary{cursor:pointer}.changes summary{font-weight:500}.change-list{max-height:220px;overflow:auto;margin-top:8px}.change{display:flex;align-items:center;gap:4px;padding:4px 0}.change button{min-width:0;padding:6px;font-size:12px;text-align:left;overflow-wrap:anywhere;min-height:32px;background:var(--wb-control);border-color:transparent}.change button:first-child{flex:1;display:flex;align-items:baseline;gap:8px}.file-kind{flex:none;color:var(--wb-muted);font-size:11px}.change small{display:block;font-size:10px;color:var(--wb-muted);margin-top:2px}
  .chat-heading{display:flex;justify-content:space-between;align-items:center;gap:8px;font-size:13px;margin-top:18px}.chat-heading strong{font-weight:600}.chat-heading strong span{font-size:12px;font-weight:400;color:var(--wb-muted);margin-left:3px}.explanation{color:var(--wb-muted);font-size:11px;line-height:1.7;margin:8px 0 12px}.chat-search{display:flex;align-items:center;gap:4px;border:1px solid var(--wb-border);border-radius:6px;padding-left:8px;color:var(--wb-muted);margin-bottom:7px}
  .chats article{padding:11px 0;border-bottom:1px solid var(--wb-border)}.chats article:last-child{border-bottom:0}.chat-link{font-size:13px;color:var(--wb-text);text-align:left;padding:0;border:0;background:transparent;line-height:1.6;overflow-wrap:anywhere}.chat-link:hover{color:var(--wb-info-text)}.chat-link :global(svg){display:inline;vertical-align:middle;color:var(--wb-muted);margin-left:3px}.chats small{display:block;font-size:11px;color:var(--wb-muted);margin-top:4px}.load-chats{width:100%;font-size:12px;min-height:34px;margin-top:10px;background:var(--wb-control)}
  .empty{text-align:center;padding:32px 16px;color:var(--wb-muted);font-size:12px}.back-to-list{display:none}.action-status{font-size:12px;color:var(--wb-muted);margin:12px 0 0}
  .wb-dialog{font:13px/1.6 var(--font-family-ui);color:var(--wb-text);background:var(--wb-panel);border:1px solid var(--wb-border);border-radius:10px;max-width:min(90vw,700px);max-height:85vh;padding:20px;color-scheme:inherit}.wb-dialog::backdrop{background:var(--wb-scrim);backdrop-filter:blur(3px)}.wb-dialog header{display:flex;align-items:center;justify-content:space-between;gap:15px;margin-bottom:14px}.wb-dialog h3{margin:0;font-size:15px;overflow-wrap:anywhere}.wb-dialog header button{min-width:30px;min-height:30px}.warning-text{font-size:12px;overflow-wrap:anywhere;border-bottom:1px solid var(--wb-border);padding-bottom:8px}.source-dialog dl{margin:0}.source-dialog dt{font-size:13px;font-weight:600;margin-top:14px}.source-dialog dd{margin:5px 0 0;font-size:12px;color:var(--wb-muted)}.source-dialog code{display:block;font-size:11px;overflow-wrap:anywhere;margin:4px 0 10px;color:var(--wb-text)}.source-dialog>p{font-size:12px;color:var(--wb-muted);border-top:1px solid var(--wb-border);padding-top:12px}
  .preview{width:94vw;max-width:1300px}.versions{display:flex;gap:8px;align-items:center;margin:15px 0;font-size:12px}.versions span{margin-left:auto;color:var(--wb-muted)}.versions button.active{border-color:var(--wb-accent)}.merge{max-height:65vh;overflow:auto;color:var(--wb-text)}.merge :global(.cm-editor){background:var(--wb-bg);color:var(--wb-text);font-size:12px}.merge :global(.cm-gutters){background:var(--wb-control);color:var(--wb-muted)}.merge :global(.cm-mergeView){max-height:65vh;overflow:auto}
  .merge :global(.cm-merge-a .cm-changedLine),.merge :global(.cm-deletedLine){background:var(--wb-danger-bg)}.merge :global(.cm-merge-b .cm-changedLine),.merge :global(.cm-insertedLine){background:var(--wb-success-bg)}.merge :global(.cm-merge-a .cm-changedText),.merge :global(.cm-deletedText){background:var(--wb-deleted-text)}.merge :global(.cm-merge-b .cm-changedText),.merge :global(.cm-insertedText){background:var(--wb-inserted-text)}.merge :global(.cm-merge-a .cm-changedLineGutter){background:var(--wb-danger-text)}.merge :global(.cm-merge-b .cm-changedLineGutter){background:var(--wb-success-text)}.merge :global(.cm-collapsedLines){background:var(--wb-control);color:var(--wb-muted)}
  @container(max-width:900px){.results-heading{align-items:flex-start;flex-direction:column;gap:2px}.filters{padding-left:12px;padding-right:12px}.detail{padding:16px}.scope-caption{flex-direction:column;gap:2px}.searchline{gap:8px}.search{flex-basis:100%}.scan-action{width:160px}}
  @media(max-width:780px){.detail-open :is(.scope-heading,.scanbar,.read-notice,.overview){display:none}.workspace{grid-template-columns:1fr}.results{border-right:0}.detail{display:none}.workspace.viewing-detail .results{display:none}.workspace.viewing-detail .detail{display:block;max-height:none}.back-to-list{display:block;margin-bottom:16px;min-height:34px;font-size:12px}}
  @container(max-width:660px){.detail-open :is(.scope-heading,.scanbar,.read-notice,.overview){display:none}.workspace{grid-template-columns:1fr}.results{border-right:0}.detail{display:none}.workspace.viewing-detail .results{display:none}.workspace.viewing-detail .detail{display:block;max-height:none}.back-to-list{display:block;margin-bottom:16px;min-height:34px;font-size:12px}.scope-tools{align-items:flex-end;flex-direction:column;gap:0}}
  @container(max-width:460px){.wb-body{width:94%;padding-top:16px}.scanbar{flex-wrap:wrap}.scan-action{width:100%;flex-direction:row;align-items:center;justify-content:space-between;margin-left:0}.scan-action small{max-width:55%;align-self:center}.overview{gap:7px}.overview button{padding:11px 9px}.overview strong{font-size:26px}.overview small{font-size:10px}.filterline{gap:6px}.row-select,.column-head{grid-template-columns:1.35fr 1fr .8fr;gap:8px}.branch{font-size:12px}.read-notice{padding:9px 10px}.detail{padding:16px}}
</style>
