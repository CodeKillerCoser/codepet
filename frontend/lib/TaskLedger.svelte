<script lang="ts">
  import { onMount, tick } from 'svelte';
  import * as Dialog from './ui/dialog/index';
  import { open } from '@tauri-apps/plugin-dialog';
  import { ChevronRight, Ellipsis } from '@lucide/svelte';
  import { taskRequest as api, currentRound, taskStatus, phases, statuses, roundStatus, scopes, results, latestResult, acceptanceReady, filterTasks, stageProgress, type Task, type Round, type Criterion, type PlanInput, type Scope, type TestResult, type SaveInput, type Receipt } from './taskLedger';
  export let workspaceHint = '';
  let tasks:Task[]=[], query='', project='', status='all', selectedId='', roundId='', busy=false, error='', message='', loading=true, mobileDetail=false;
  let heading:HTMLHeadingElement, lastButton:HTMLButtonElement|undefined;
  let editor:Dialog.ModalHandle, verifyDialog:Dialog.ModalHandle, closeDialog:Dialog.ModalHandle, setupDialog:Dialog.ModalHandle;
  let editTask:Task|null=null, title='', objective='', description='', workspace='', kind='feature', phase:Task['rounds'][number]['phase']='in_progress', note='', commits='', delivery='', summary='', areas='', criteria:Criterion[]=[], includePlan=false, originalPlan='';
  let verifyTask:Task|null=null, verifyRound:Round|null=null, stage:Scope='release', target='', environment='', checks:Record<string,{result:TestResult;evidence:string}>={};
  let closeTask:Task|null=null, closeMode='closed', closeAccepted=false, closeReason='';
  let setup={config:'',instructions:''}, copied='';
  let moreOpen=false, moreTrigger:HTMLElement;
  let pendingSave:{fingerprint:string;operationId:string}|null=null;
  $: filtered=filterTasks(tasks,query,project,status);
  $: selected=filtered.find(t=>t.id===selectedId)??filtered[0];
  $: round=selected?.rounds.find(r=>r.id===roundId)??(selected&&currentRound(selected));
  $: isCurrent=!!selected&&round?.id===selected.currentRoundId;
  $: plan=round?.plans.at(-1);
  $: code=round?.codeRefs.at(-1);
  $: localProgress=round&&stageProgress(round,'local');
  $: releaseProgress=round&&stageProgress(round,'release');
  $: failedCriterionIds=new Set(plan?.criteria.filter(c=>round&&(['local','release'] as const).some(stage=>(c.scope==='both'||c.scope===stage)&&latestResult(round,c,stage).result==='failed')).map(c=>c.id)??[]);
  $: linkedCommits=round?[...new Map(round.codeRefs.flatMap(observation=>observation.commits.map(commit=>[commit.sha,{...commit,observedAt:observation.observedAt}] as const))).values()]:[];
  $: projects=[...new Map(tasks.map(t=>[t.projectId,t.projectName])).entries()];
  const fmt=(time:number)=>new Date(time).toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false});
  const blank=():Criterion=>({id:'',title:'',precondition:'',steps:'',expected:'',scope:'both',required:true});
  async function load() { loading=true;error='';try{tasks=await api<Task[]>('list');}catch(e){error=String(e);}finally{loading=false;} }
  function canRefresh(){return !busy&&![editor,verifyDialog,closeDialog,setupDialog].some(d=>d?.isOpen());}
  onMount(()=>{void load();const focus=()=>{if(canRefresh())void load();};window.addEventListener('focus',focus);return()=>window.removeEventListener('focus',focus);});
  async function select(task:Task,button:HTMLButtonElement){selectedId=task.id;roundId='';mobileDetail=true;lastButton=button;await tick();heading?.focus({preventScroll:true});}
  async function back(){mobileDetail=false;await tick();lastButton?.focus({preventScroll:true});}
  async function chooseDirectory(){const path=await open({directory:true,multiple:false});if(typeof path==='string')workspace=path;}
  function edit(task:Task|null){
    editTask=task?structuredClone(task):null;title=task?.title??'';objective=task?.objective??'';description=task?.description??'';workspace=task?.workspace??workspaceHint;kind=task?.kind??'feature';phase=task?currentRound(task).phase:'in_progress';note='';commits='';delivery='';
    const p=task?currentRound(task).plans.at(-1):undefined;
    summary=p?.summary??'';areas=p?.affectedAreas.join('\n')??'';criteria=p?structuredClone(p.criteria):[blank()];includePlan=!!p;originalPlan=JSON.stringify(p?{summary:p.summary,affectedAreas:p.affectedAreas,criteria:p.criteria}:null);error='';editor.showModal();
  }
  async function save(input:SaveInput){
    const fingerprint=JSON.stringify(input);
    if(pendingSave?.fingerprint!==fingerprint)pendingSave={fingerprint,operationId:crypto.randomUUID()};
    const receipt=await api<Receipt>('save',{input:{...input,operationId:pendingSave.operationId}});
    pendingSave=null;
    selectedId=receipt.taskId;roundId='';await load();return receipt;
  }
  async function saveEditor(){
    busy=true;error='';try{
      const input:SaveInput={title,objective,description,kind,phase,note:note.trim()||undefined,commits:commits.split(/[\s,]+/).filter(Boolean),delivery:delivery.trim()||undefined};
      if(editTask){input.taskId=editTask.id;input.expectedRevision=editTask.revision;if(workspace!==editTask.workspace)input.workspace=workspace;}else input.workspace=workspace;
      const p:PlanInput={summary,affectedAreas:areas.split('\n').map(s=>s.trim()).filter(Boolean),criteria};
      if(includePlan&&JSON.stringify(p)!==originalPlan)input.plan=p;
      await save(input);if(!editTask)status='all';editor.close();message='任务已保存';
    }catch(e){error=String(e);}finally{busy=false;}
  }
  function startVerification(){if(!selected||!round||!plan)return;verifyTask=structuredClone(selected);verifyRound=structuredClone(round);stage=plan.criteria.every(c=>c.scope==='local')?'local':'release';target='';environment='';checks=Object.fromEntries(plan.criteria.map(c=>[c.id,{result:'not_tested',evidence:''}]));error='';verifyDialog.showModal();}
  async function saveVerification(){
    if(!verifyTask||!verifyRound)return;busy=true;error='';try{
      const p=verifyRound.plans.at(-1)!;
      await save({taskId:verifyTask.id,expectedRevision:verifyTask.revision,verification:{planRevision:p.revision,stage,target,environment,results:p.criteria.filter(c=>c.scope==='both'||c.scope===stage).map(c=>({criterionId:c.id,...checks[c.id]}))},note:`人工记录${scopes[stage]}验收：${target}`});
      verifyDialog.close();message='验收记录已保存；历史结果已保留';
    }catch(e){error=String(e);}finally{busy=false;}
  }
  function startClose(reopen=false){if(!selected)return;closeTask=structuredClone(selected);closeMode=reopen?'reopen':'closed';closeAccepted=!reopen&&acceptanceReady(currentRound(selected));closeReason='';error='';closeDialog.showModal();}
  async function finish(){
    if(!closeTask)return;busy=true;error='';try{
      if(closeMode==='reopen')await save({taskId:closeTask.id,expectedRevision:closeTask.revision,reopenReason:closeReason});
      else{await api('close',{taskId:closeTask.id,expectedRevision:closeTask.revision,outcome:closeAccepted?'accepted':'closed',reason:closeReason.trim()||(closeAccepted?'验收通过，结束本轮':'手动结束本轮')});await load();}
      closeDialog.close();status='all';message=closeMode==='reopen'?'已开启新一轮，上一轮结果已保留':'本轮已结束';
    }catch(e){error=String(e);}finally{busy=false;}
  }
  async function showSetup(){moreOpen=false;moreTrigger?.focus();error='';try{setup=await api('setup');copied='';setupDialog.showModal();}catch(e){error=String(e);}}
  async function copy(text:string,label:string){try{await navigator.clipboard.writeText(text);copied=`已复制${label}`;}catch{copied='请选中文本复制';}}
  async function openChat(threadId:string){try{await api('openChat',{taskId:selected?.id,threadId});}catch(e){error=String(e);}}
</script>

<svelte:window on:click={event=>{if(moreOpen&&!(event.target instanceof Element&&event.target.closest('.ledger-more')))moreOpen=false;}} on:keydown={event=>{if(event.key==='Escape'&&moreOpen){event.preventDefault();moreOpen=false;moreTrigger?.focus();}}}/>

<section class="task-ledger" aria-label="任务与验收">
  <div class="toolbar">
    <div class="filters" role="search" aria-label="筛选任务">
      <input aria-label="搜索任务" placeholder="搜索任务、背景或复发原因" bind:value={query}/>
      <select aria-label="任务项目" bind:value={project}><option value="">全部项目</option>{#each projects as [id,name]}<option value={id}>{name}</option>{/each}</select>
      <select aria-label="任务状态" bind:value={status}><option value="all">全部状态</option>{#each Object.entries(statuses) as [value,label]}<option {value}>{label}</option>{/each}</select>
    </div>
    <div class="toolbar-actions">
      <button class="quiet" disabled={busy||loading} on:click={load}>刷新任务</button><button class="primary" disabled={busy} on:click={()=>edit(null)}>新建任务</button>
      <details class="ledger-more" open={moreOpen}><summary aria-label="更多任务操作" title="更多任务操作" aria-expanded={moreOpen} on:click|preventDefault={()=>moreOpen=!moreOpen} bind:this={moreTrigger}><Ellipsis size={16}/></summary><div class="more-menu"><button class="quiet" on:click={showSetup}>接入 Codex</button></div></details>
    </div>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if message}<p class="message" role="status">{message}</p>{/if}
  <div class="ledger-layout" class:detail-open={mobileDetail}>
    <section class="task-list" aria-label="任务列表">
      <div class="list-heading"><small class="result-count">{filtered.length} 个结果</small></div>
      <div class="task-rows">
        {#each filtered as task(task.id)}
          <button class="task-row" class:selected={selected?.id===task.id} aria-pressed={selected?.id===task.id} on:click={e=>select(task,e.currentTarget)}>
            <span class="row-top"><strong>{task.title}</strong><span class="row-phase status-badge" data-status={roundStatus(currentRound(task))}>{taskStatus(task)}</span></span>
            <span class="row-context"><span>{task.projectName} · {task.rounds.length} 次处理</span><small>{fmt(task.updatedAt)}</small></span>
          </button>
        {:else}<div class="empty">{loading?'读取任务…':tasks.length?'没有符合条件的任务':'还没有登记任务'}{#if !loading&&!tasks.length}<p>连接 Codex 后，在方案和实施节点自动登记；也可以手动新建。</p>{/if}</div>{/each}
      </div>
    </section>
    <aside class="task-detail" aria-label="任务详情">
      {#if selected&&round}
        <button class="back quiet" on:click={back}>← 返回任务列表</button>
        <h3 tabindex="-1" bind:this={heading}>{selected.title}</h3>
        <p class="detail-objective">{round.objective}</p>
        <div class="round-status"><span class="phase-label status-badge" data-status={roundStatus(round)}>{statuses[roundStatus(round)]}</span>{#if selected.rounds.length>1}<select aria-label="处理轮次" bind:value={roundId}><option value="">第 {currentRound(selected).number} 次处理 · 当前</option>{#each selected.rounds.slice(0,-1).reverse() as r}<option value={r.id}>第 {r.number} 次处理 · {statuses[roundStatus(r)]}</option>{/each}</select>{/if}<span class="round-reason">{round.openedReason}</span></div>
        {#if round.closure}<p class="closure">{fmt(round.closure.closedAt)} · {round.closure.reason}</p>{/if}
        <div class="progress-line" aria-label="当前轮次验收进度">
          <div class="stage-progress"><span>本地验证</span><strong>{localProgress?.total?localProgress.passed:'—'}{#if localProgress?.total}<small> / {localProgress.total}</small>{/if}</strong>{#if localProgress?.failed}<small class="failure">{localProgress.failed} 项失败</small>{/if}</div>
          <div class="stage-progress"><span>发布验证</span><strong>{releaseProgress?.total?releaseProgress.passed:'—'}{#if releaseProgress?.total}<small> / {releaseProgress.total}</small>{/if}</strong>{#if releaseProgress?.failed}<small class="failure">{releaseProgress.failed} 项失败</small>{/if}</div>
          {#if isCurrent}<div class="actions">{#if !round.closure}<button class:primary={!!plan} class:quiet={!plan} disabled={!plan} on:click={startVerification}>记录人工验收</button><button class:quiet={!!plan} class:primary={!plan} on:click={()=>edit(selected)}>编辑与记录进展</button><button class="quiet end-round" on:click={()=>startClose()}>结束任务</button>{:else}<button class="primary" on:click={()=>startClose(true)}>重新开启</button>{/if}</div>{/if}
        </div>

        <section class="acceptance-section" aria-label="方案与验收">
          <div class="section-heading"><h4>方案与验收</h4><small>{plan?`${plan.criteria.length} 项 · 清单第 ${plan.revision} 版`:'待补充验收计划'}</small></div>
          {#if plan}
            <div class="plan-intro"><p class="plan-summary">{plan.summary}</p>{#if plan.affectedAreas.length}<p class="muted">影响范围：{plan.affectedAreas.join('、')}</p>{/if}</div>
            {#key `${selected.id}-${round.id}`}
            <div class="criteria">
              {#each plan.criteria as criterion,i}
                <details class="criterion" open={failedCriterionIds.size?failedCriterionIds.has(criterion.id):i===0}>
                  <summary>
                    <ChevronRight class="fold-icon" size={12}/><span class="criterion-number">{String(i+1).padStart(2,'0')}</span>
                    <span class="criterion-name"><strong>{criterion.title}</strong><small>{criterion.required?'必验':'可选'} · {scopes[criterion.scope]}</small></span>
                    <span class="test-status">{#each criterion.scope==='both'?['local','release']: [criterion.scope] as s}{@const result=latestResult(round,criterion,s as Scope)}<span class:failure={result.result==='failed'} title={result.evidence??''}>{s==='local'?'本地':'发布'} <b>{results[result.result]}</b>{#if result.actor}<small>{result.actor==='human'?'人工':'Codex 自测'}</small>{/if}</span>{/each}</span>
                  </summary>
                  <dl class="criterion-body">
                    {#if criterion.precondition}<div><dt>前提</dt><dd>{criterion.precondition}</dd></div>{/if}
                    <div><dt>操作</dt><dd>{criterion.steps}</dd></div><div><dt>预期</dt><dd>{criterion.expected}</dd></div>
                  </dl>
                </details>
              {/each}
            </div>
            {/key}
          {:else}<p class="muted empty-plan">方案确定后登记验收点，作为本地验证和发布后复验的依据。</p>{/if}
          {#if round.plans.length>1}<details class="plan-history"><summary>历史清单修订 <small>{round.plans.length-1} 版</small></summary>{#each round.plans.slice(0,-1).reverse() as p}<article class="history-item"><strong>第 {p.revision} 版 · {fmt(p.createdAt)}</strong><p>{p.summary}</p>{#each p.criteria as c}<p class="muted">{c.title}：{c.steps} → {c.expected}</p>{/each}</article>{/each}</details>{/if}
        </section>

        <div class="supporting-details">
          <details class="meta-section code-section">
            <summary><ChevronRight class="fold-icon" size={12}/><span>代码与交付</span><span class="meta-preview"><code>{code?.branch??'分支未知'}</code><small>{linkedCommits.length?`${linkedCommits.length} 个关联提交`:'未关联提交'}</small></span></summary>
            <div class="meta-body">{#if code}<p><code>{code.branch??'游离 HEAD / 分支未知'}{code.head?` · ${code.head.slice(0,10)}`:''}</code></p><p class="muted">{code.dirty===null?'工作树状态未知':code.dirty?'观测时工作树有未提交改动（共享状态）':'观测时工作树干净'} · {fmt(code.observedAt)}</p><p class="path">{code.workspace}</p>{#if code.error}<p class="muted">{code.error}</p>{/if}{:else}<p class="muted">暂无 Git 观测</p>{/if}
              {#if round.deliveryFacts}<div class="delivery-facts">{#each round.deliveryFacts.items as fact}<p class="muted">{fact.scope==='shared_worktree'?'共享工作树':'关联提交'}：{fact.label} · {fmt(fact.observedAt)}</p>{/each}<p class="muted">{round.deliveryFacts.note}</p></div>{/if}
              {#each linkedCommits as commit}<div class="commit-group"><code>{commit.sha.slice(0,12)}</code><small>{commit.inCachedUpstream===null?'上游状态未知':commit.inCachedUpstream?'在本地缓存上游中':'未在本地缓存上游中'} · {fmt(commit.observedAt)}</small></div>{/each}
              {#if !linkedCommits.length}<p class="muted">尚未明确关联任务提交；工作树状态由同一目录中的任务共享。</p>{/if}
              {#each round.deliveries as delivery}<p class="preserve">交付登记：{delivery}</p>{/each}
            </div>
          </details>
          {#if round.description}<details class="meta-section background"><summary><ChevronRight class="fold-icon" size={12}/><span>背景与现象</span><span class="meta-preview">{round.description}</span></summary><p class="meta-body preserve">{round.description}</p></details>{/if}
          <details class="meta-section verification-section">
            <summary><ChevronRight class="fold-icon" size={12}/><span>验收记录 <small>{round.verifications.length}</small></span><span class="meta-preview">{round.verifications.at(-1)?.target??'暂无实际验收'}</span></summary>
            <div class="meta-body">{#each [...round.verifications].reverse() as run}<details class="verification"><summary>{run.actor==='human'?'人工验收':'Codex 自测'} · {scopes[run.stage]} · {run.target}<small>清单第 {run.planRevision} 版 · {fmt(run.createdAt)}</small></summary><p class="muted">{run.environment}</p>{#each run.results as result}<p class:failure={result.result==='failed'}>{round.plans.find(p=>p.revision===run.planRevision)?.criteria.find(c=>c.id===result.criterionId)?.title??'历史验收项'}：{results[result.result]}</p><p class="preserve muted">{result.evidence}</p>{/each}{#if run.workspaceObservation?.dirty}<p class="muted">当时工作区包含未提交修改；HEAD 不代表完整被测内容。</p>{/if}</details>{:else}<p class="muted">尚未记录实际验收。</p>{/each}</div>
          </details>
          <details class="meta-section sources-section">
            <summary><ChevronRight class="fold-icon" size={12}/><span>关联对话 <small>{round.sources.length}</small></span><span class="meta-preview">{round.sources.length?'Codex':'尚未关联'}</span></summary>
            <div class="meta-body">{#each round.sources as source}<button class="chat quiet" disabled={!source.verified} on:click={()=>openChat(source.threadId)}>Codex · {source.threadId.slice(0,12)} {source.verified?'↗':'· 待核实'}</button>{:else}<p class="muted">尚未关联对话，任务记录仍可独立使用。</p>{/each}</div>
          </details>
          <details class="meta-section timeline-section">
            <summary><ChevronRight class="fold-icon" size={12}/><span>本轮时间线 <small>{round.events.length}</small></span><span class="meta-preview">{round.events.at(-1)?.summary??'暂无进展记录'}</span></summary>
            <ol class="timeline meta-body">{#each [...round.events].reverse() as event}<li><p>{event.summary}</p><small>{fmt(event.at)} · {event.actor==='human'?'人工':'Codex'}</small></li>{/each}</ol>
          </details>
        </div>
      {:else}<div class="empty">选择任务查看方案、提交与验收历史</div>{/if}
    </aside>
  </div>
</section>

<Dialog.Modal title={editTask?'编辑与记录进展':'新建任务'} class="ledger-dialog" bind:this={editor} onCancel={e=>{if(busy)e.preventDefault();}}><form on:submit|preventDefault={saveEditor}><fieldset class="form-controls" disabled={busy}><header><h3>{editTask?'编辑与记录进展':'新建任务'}</h3><button type="button" aria-label="关闭任务编辑" disabled={busy} on:click={()=>editor.close()}>×</button></header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <label>标题<input required maxlength="160" bind:value={title}/></label><label>任务内容<textarea required maxlength="4000" rows="3" bind:value={objective} placeholder="要解决什么问题，达到什么结果"></textarea></label>
  <div class="form-row"><label>任务状态<select bind:value={phase}>{#each Object.entries(phases) as [value,label]}<option {value}>{label}</option>{/each}</select></label></div>
  <label>工作目录<div class="directory"><input required bind:value={workspace} placeholder="项目或工作树的绝对路径"/><button type="button" on:click={chooseDirectory}>选择</button></div></label><label>背景描述（可选）<textarea maxlength="8000" rows="2" bind:value={description}></textarea></label>
  <label>进展摘要（可选）<textarea rows="2" maxlength="2000" bind:value={note}></textarea></label><label>本任务关联的 commit（可选）<input bind:value={commits} placeholder="填写 SHA，以空格或逗号分隔"/></label>
  <label>{phase==='published'&&(!editTask||currentRound(editTask).phase!=='published')?'发布版本或记录':'构建或发布引用（可选）'}<input required={phase==='published'&&(!editTask||currentRound(editTask).phase!=='published')} bind:value={delivery} placeholder="版本、构建链接或发布说明"/></label>
  <label class="checkbox"><input type="checkbox" bind:checked={includePlan}/>登记或修订验收计划</label>
  {#if includePlan}<label>方案摘要<textarea required rows="2" bind:value={summary}></textarea></label><label>影响范围（每行一项）<textarea rows="2" bind:value={areas}></textarea></label>
    {#each criteria as criterion,i}<fieldset><legend>验收点 {i+1}</legend><label>检查点<input required bind:value={criterion.title}/></label><label>前置条件（可选）<input bind:value={criterion.precondition}/></label><label>操作步骤<textarea required rows="2" bind:value={criterion.steps}></textarea></label><label>预期结果<textarea required rows="2" bind:value={criterion.expected}></textarea></label><div class="form-row"><label>适用阶段<select bind:value={criterion.scope}>{#each Object.entries(scopes) as [value,label]}<option {value}>{label}</option>{/each}</select></label><label class="checkbox"><input type="checkbox" bind:checked={criterion.required}/>必验项</label><button type="button" disabled={criteria.length===1} on:click={()=>criteria=criteria.filter((_,n)=>n!==i)}>移除</button></div></fieldset>{/each}
    <button type="button" disabled={criteria.length>=30} on:click={()=>criteria=[...criteria,blank()]}>添加验收点</button>
  {/if}<footer><span>已有验收与清单历史会保留。</span><button class="primary" disabled={busy}>{busy?'保存中…':'保存任务'}</button></footer>
</fieldset></form></Dialog.Modal>

<Dialog.Modal title={'记录人工验收'} class="ledger-dialog" bind:this={verifyDialog} onCancel={e=>{if(busy)e.preventDefault();}}><form on:submit|preventDefault={saveVerification}><fieldset class="form-controls" disabled={busy}><header><h3>记录人工验收</h3><button type="button" aria-label="关闭人工验收" disabled={busy} on:click={()=>verifyDialog.close()}>×</button></header>{#if error}<p class="error" role="alert">{error}</p>{/if}
  <label>验收阶段<select bind:value={stage}><option value="local">本地</option><option value="release">发布后</option></select></label><label>实际验收版本<input required bind:value={target} placeholder="已安装版本、commit，或未提交工作区说明"/></label><label>验收环境<input required bind:value={environment} placeholder="操作系统、运行环境"/></label>
  {#each verifyRound?.plans.at(-1)?.criteria.filter(c=>c.scope==='both'||c.scope===stage)??[] as criterion}<fieldset><legend>{criterion.title}</legend><p>{criterion.steps} → {criterion.expected}</p><label>结果<select bind:value={checks[criterion.id].result}>{#each Object.entries(results) as [value,label]}<option {value}>{label}</option>{/each}</select></label><label>证据或说明<textarea required rows="2" bind:value={checks[criterion.id].evidence} placeholder="实际观察结果、日志或截图位置"></textarea></label></fieldset>{/each}
  <footer><span>只记录本次结果，不覆盖此前验收。</span><button class="primary" disabled={busy||!verifyRound?.plans.at(-1)?.criteria.some(c=>c.scope==='both'||c.scope===stage)}>保存验收记录</button></footer>
</fieldset></form></Dialog.Modal>

<Dialog.Modal title={closeMode==='reopen'?'重新开启任务':'结束任务'} class="ledger-dialog small-dialog" bind:this={closeDialog} onCancel={e=>{if(busy)e.preventDefault();}}><form on:submit|preventDefault={finish}><fieldset class="form-controls" disabled={busy}><header><h3>{closeMode==='reopen'?'重新开启任务':'结束任务'}</h3><button type="button" aria-label="关闭结束操作" disabled={busy} on:click={()=>closeDialog.close()}>×</button></header>{#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if closeMode!=='reopen'}<label class="checkbox"><input type="checkbox" bind:checked={closeAccepted} disabled={!closeTask||!acceptanceReady(currentRound(closeTask))}/>验收已通过</label><p class="muted">未完成验收也可以结束任务，已有检查结果会保留。</p>{:else}<p>新一轮从“进行中”开始，原来的处理与验收历史会保留。</p>{/if}
  <label>{closeMode==='reopen'?'复发或重新处理的原因':'结束说明（可选）'}<textarea required={closeMode==='reopen'} rows="3" maxlength="2000" bind:value={closeReason}></textarea></label><footer><button class="primary" disabled={busy}>{closeMode==='reopen'?'开启新一轮':'确认结束'}</button></footer>
</fieldset></form></Dialog.Modal>

<Dialog.Modal title="连接 Codex 任务登记" class="ledger-dialog" returnFocus={moreTrigger} bind:this={setupDialog}><header><h3>连接 Codex 任务登记</h3><button aria-label="关闭接入说明" on:click={()=>setupDialog.close()}>×</button></header><p>将下面配置替换 config.toml 中的 codepet_tasks 条目，然后在新对话中使用。本机 HTTP 服务由 Code Pet 后台提供，关闭窗口不影响连接；退出应用后停止。</p><label>MCP 配置<textarea readonly rows="5" value={setup.config}></textarea></label><button on:click={()=>copy(setup.config,'配置')}>复制配置</button><p>将下面的简短约定加入项目 AGENTS.md，让登记集中在关键节点。</p><label>项目约定<textarea readonly rows="6" value={setup.instructions}></textarea></label><button on:click={()=>copy(setup.instructions,'项目约定')}>复制项目约定</button><p role="status">{copied}</p></Dialog.Modal>

<style>
  .task-ledger{width:100%;margin:0;padding:0;color:var(--wb-text);font:13px/1.6 var(--font-family-ui);min-width:0}
  *{box-sizing:border-box}button,input,textarea,select{font:inherit;color:inherit;border:1px solid var(--wb-border);border-radius:5px;background:var(--wb-panel);min-width:0}
  button{cursor:pointer;padding:5px 9px;min-height:30px}button:hover{border-color:var(--wb-border-strong)}button:disabled{opacity:.5;cursor:default}
  :is(button,input,textarea,select,summary):focus-visible{outline:2px solid var(--color-focus-ring);outline-offset:2px}
  .primary{background:var(--wb-primary);border-color:var(--wb-primary);color:var(--wb-on-primary);font-weight:500}.quiet{background:transparent;border-color:transparent;color:var(--wb-muted)}.quiet:hover{background:var(--wb-control);border-color:transparent;color:var(--wb-text)}
  input,textarea,select{padding:6px 8px}textarea{resize:vertical;width:100%}
  .toolbar{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:14px}.toolbar-actions{display:flex;align-items:center;gap:5px;flex:none}.toolbar button{font-size:12px;white-space:nowrap}
  .filters{display:grid;grid-template-columns:minmax(170px,1fr) minmax(104px,132px) minmax(96px,116px);gap:8px;flex:1;min-width:0}.filters :is(input,select){width:100%;height:32px;font-size:12px;background:transparent}.list-heading{padding:10px 10px 7px}.result-count{font-size:11px}
  .ledger-more{position:relative;flex:none}.ledger-more>summary{display:flex;align-items:center;justify-content:center;list-style:none;width:30px;height:30px;border-radius:5px;color:var(--wb-muted);cursor:pointer}.ledger-more>summary::-webkit-details-marker{display:none}.ledger-more>summary:hover,.ledger-more[open]>summary{background:var(--wb-control);color:var(--wb-text)}.more-menu{position:absolute;right:0;top:calc(100% + 6px);z-index:5;min-width:136px;padding:5px;border:1px solid var(--wb-border);border-radius:7px;background:var(--wb-panel);box-shadow:0 8px 24px var(--wb-scrim)}.more-menu button{width:100%;text-align:left}
  .error{background:var(--wb-danger-bg);color:var(--wb-danger-text);padding:8px 10px;border-radius:5px;overflow-wrap:anywhere}.message{font-size:11px;color:var(--wb-muted);margin:0 0 10px}
  .ledger-layout{display:grid;grid-template-columns:minmax(230px,.8fr) minmax(0,1.4fr);border-top:1px solid var(--wb-border);min-height:440px}
  .task-list{border-right:1px solid var(--wb-border);min-width:0;padding-right:10px}
  .muted,small{color:var(--wb-muted);font-size:11px}.task-rows{max-height:calc(100dvh - 260px);overflow:auto;padding:0 0 8px}.task-row{width:100%;display:flex;flex-direction:column;align-items:stretch;gap:5px;border:0;border-bottom:1px solid var(--wb-border);border-radius:0;text-align:left;padding:12px 10px;background:transparent}.task-row:hover{background:var(--wb-control)}.task-row.selected{background:var(--wb-control);box-shadow:inset 2px 0 var(--wb-border-strong)}.task-row.selected strong{font-weight:600}.row-top{display:flex;justify-content:space-between;align-items:flex-start;gap:12px}.row-top>strong{font-size:13px;font-weight:500;line-height:1.55;overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}.row-phase{flex:none;font-size:10px;color:var(--wb-muted);padding-top:2px;white-space:nowrap}.row-context{display:flex;align-items:baseline;justify-content:space-between;gap:8px;flex-wrap:wrap;font-size:10px;color:var(--wb-muted)}.row-context small{font-size:10px}
  .task-detail{min-width:0;padding:16px 0 12px 24px;max-height:calc(100dvh - 190px);overflow:auto;scrollbar-gutter:stable}.round-status select{font-size:11px;padding:3px 6px;max-width:100%;border-color:transparent;background:transparent}.round-status select:hover{border-color:var(--wb-border)}
  .task-detail h3{font-size:20px;line-height:1.45;letter-spacing:-.02em;font-weight:600;margin:0 0 7px;overflow-wrap:anywhere}.task-detail h3:focus{outline:none}.detail-objective{font-size:13px;line-height:1.75;margin:0;white-space:pre-wrap;overflow-wrap:anywhere;max-width:68ch}.round-status{display:flex;align-items:baseline;flex-wrap:wrap;gap:6px 12px;margin:10px 0 0;font-size:11px}.phase-label{font-weight:600;white-space:nowrap}.round-reason{color:var(--wb-muted);overflow-wrap:anywhere}.closure{font-size:12px;color:var(--wb-muted);margin:10px 0 0}
  .status-badge{display:inline-flex;align-items:center;justify-content:center;padding:2px 7px;border-radius:4px;border:0;font-size:11px;font-weight:500;line-height:1.5;white-space:nowrap}
  .status-badge[data-status="in_progress"]{background:var(--wb-info-bg);color:var(--wb-info-strong-text)}
  .status-badge[data-status="ready_for_delivery"]{background:var(--wb-warning-bg);color:var(--wb-warning-strong-text)}
  .status-badge[data-status="published"]{background:var(--wb-success-bg);color:var(--wb-success-text)}
  .status-badge[data-status="closed"]{background:var(--wb-neutral-bg);color:var(--wb-neutral-text)}
  .delivery-facts{margin:8px 0}
  .progress-line{display:grid;grid-template-columns:auto auto minmax(0,1fr);align-items:center;gap:20px;border-block:1px solid var(--wb-border);margin:16px 0 0;padding:12px 0}.stage-progress{display:grid;grid-template-columns:auto auto;align-items:baseline;gap:2px 9px}.stage-progress>span{font-size:11px;color:var(--wb-muted)}.stage-progress>strong{font-size:17px;font-weight:600;line-height:1.3;font-variant-numeric:tabular-nums}.stage-progress>strong>small{font-size:11px;font-weight:400}.stage-progress>small{grid-column:1/-1}.actions{display:flex;justify-content:flex-end;gap:3px;flex-wrap:wrap}.actions button{font-size:11px;padding:5px 7px;white-space:nowrap}.end-round{font-size:10px!important}
  .acceptance-section{padding:18px 0 16px}.section-heading{display:flex;align-items:baseline;justify-content:space-between;gap:10px}.section-heading h4{font-size:13px;font-weight:600;margin:0}.section-heading>small{font-size:10px;white-space:nowrap}.plan-intro{margin:10px 0 12px}.plan-summary{font-size:12px;line-height:1.7;margin:0;white-space:pre-wrap;overflow-wrap:anywhere}.plan-intro>.muted{margin:3px 0 0;font-size:10px}.criteria{border-top:1px solid var(--wb-border)}.criterion{border-bottom:1px solid var(--wb-border)}.criterion>summary{display:grid;grid-template-columns:12px 18px minmax(0,1fr) auto;gap:7px;align-items:center;list-style:none;padding:11px 0;cursor:pointer}.criterion>summary::-webkit-details-marker,.meta-section>summary::-webkit-details-marker{display:none}.criterion-number{font-size:10px;color:var(--wb-muted);font-variant-numeric:tabular-nums}.criterion-name{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;min-width:0}.criterion-name>strong{font-size:12px;font-weight:500;overflow-wrap:anywhere}.criterion-name>small{font-size:10px}.test-status{display:flex;align-items:baseline;gap:14px;font-size:10px;color:var(--wb-muted)}.test-status>span{display:flex;gap:4px;align-items:baseline;white-space:nowrap}.test-status b{font-weight:500;color:var(--wb-text)}.test-status small{font-size:9px}.test-status .failure b,.failure{color:var(--wb-danger-text)}
  .criterion-body{margin:0 0 12px 44px;padding-left:10px;border-left:1px solid var(--wb-border);font-size:12px}.criterion-body>div{display:grid;grid-template-columns:30px minmax(0,1fr);gap:10px;margin:5px 0}.criterion-body dt{color:var(--wb-muted);font-size:11px}.criterion-body dd{margin:0;white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.7}.plan-history{margin-top:10px}.plan-history summary{font-size:11px;color:var(--wb-muted);cursor:pointer}.plan-history summary small{margin-left:6px}.empty-plan{padding:8px 0}.history-item{padding:8px 0}.history-item p{font-size:12px;margin:5px 0}
  .supporting-details{border-top:1px solid var(--wb-border)}.meta-section{border-bottom:1px solid var(--wb-border)}.meta-section>summary{display:flex;align-items:center;gap:7px;list-style:none;min-height:38px;padding:9px 0;cursor:pointer;font-size:11px}.meta-section>summary>span:first-of-type{flex:none;font-weight:500}.meta-section>summary>span>small{font-size:10px;font-weight:400;margin-left:5px}.meta-preview{display:flex;align-items:baseline;justify-content:flex-end;gap:8px;min-width:0;flex:1;margin-left:12px;color:var(--wb-muted);font-size:10px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.meta-preview:not(:has(code)){display:block;text-align:right}.meta-preview code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.meta-preview small{white-space:nowrap;font-size:10px}.meta-body{padding:0 0 12px 19px;margin:0;font-size:12px}.meta-body p{margin:5px 0}.preserve{white-space:pre-wrap;overflow-wrap:anywhere}code,.path{font:11px/1.6 var(--font-family-code);overflow-wrap:anywhere}.path{color:var(--wb-muted)}.commit-group{display:flex;align-items:baseline;flex-wrap:wrap;gap:8px;margin:8px 0}.verification{padding:8px 0;border-top:1px solid var(--wb-border)}.verification:first-child{border-top:0}.verification summary{font-size:12px;cursor:pointer;overflow-wrap:anywhere}.verification summary small{display:block;font-size:10px;margin:3px 0 0 14px}.chat{font-size:11px}.timeline{list-style:none;margin-left:5px;padding-left:13px;border-left:1px solid var(--wb-border)}.timeline li{padding:0 0 10px 5px}.timeline p{font-size:11px;margin:0}.timeline small{font-size:10px}.meta-section :global(.fold-icon),.criterion :global(.fold-icon){color:var(--wb-muted);flex:none;transition:transform .12s ease}.meta-section[open]>summary :global(.fold-icon),.criterion[open]>summary :global(.fold-icon){transform:rotate(90deg)}
  .empty{padding:36px 16px;text-align:center;font-size:12px;color:var(--wb-muted)}.empty p{font-size:11px;line-height:1.7}.back{display:none}
  :global(.ledger-dialog){background:var(--wb-panel);color:var(--wb-text);font:13px/1.6 var(--font-family-ui);border:1px solid var(--wb-border);border-radius:10px;width:min(650px,92vw);max-height:88vh;padding:22px;color-scheme:inherit;overflow:auto}:global(.ledger-dialog) header{display:flex;justify-content:space-between;align-items:center;margin-bottom:15px}:global(.ledger-dialog) h3{font-size:16px;margin:0}:global(.ledger-dialog) label{display:flex;flex-direction:column;gap:5px;margin:12px 0;font-size:12px}:global(.ledger-dialog) .checkbox{flex-direction:row;align-items:center;gap:8px}:global(.ledger-dialog) .form-row{display:flex;gap:12px;align-items:center;flex-wrap:wrap}.form-row>label{flex:1;min-width:100px}.directory{display:flex;gap:8px}.directory input{flex:1}:global(.ledger-dialog) fieldset{margin:14px 0;padding:12px;border:1px solid var(--wb-border);border-radius:6px}:global(.ledger-dialog) .form-controls{border:0;padding:0;margin:0;min-width:0}:global(.ledger-dialog) legend{font-size:12px;font-weight:600}:global(.ledger-dialog) footer{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-top:20px;border-top:1px solid var(--wb-border);padding-top:15px}:global(.ledger-dialog) footer span{font-size:11px;color:var(--wb-muted)}:global(.small-dialog){width:min(470px,92vw)}
  @container(max-width:1050px){.progress-line{grid-template-columns:1fr 1fr;gap:12px 20px}.stage-progress{display:flex;gap:8px}.actions{grid-column:1/-1;justify-content:flex-start;gap:6px}.test-status{gap:9px}.criterion-name{row-gap:1px}}
  @container(max-width:850px){.toolbar{flex-wrap:wrap;gap:8px}.filters{flex-basis:100%}.toolbar-actions{margin-left:auto}}
  @container(max-width:700px){.toolbar button{font-size:11px;padding:5px 6px}.toolbar-actions{gap:1px}.ledger-layout{grid-template-columns:minmax(0,1fr);min-height:360px}.task-detail{display:none;max-height:none;padding:14px 0}.ledger-layout.detail-open .task-detail{display:block}.ledger-layout.detail-open .task-list{display:none}.task-list{border-right:0;padding:0}.task-rows{max-height:none}.back{display:block;margin:0 0 9px;padding-left:0}.task-detail h3{font-size:18px}.progress-line{margin-top:13px}.criterion>summary{grid-template-columns:12px 18px minmax(0,1fr)}.test-status{grid-column:3;gap:14px}.criterion-body{margin-left:37px;padding-left:8px}.meta-preview{margin-left:5px}.meta-preview code{max-width:140px}.meta-preview small{display:none}}
  @container(max-width:440px){.filters{grid-template-columns:1fr 1fr}.filters>input{grid-column:1/-1}.toolbar{margin-bottom:8px}.task-ledger h3{font-size:18px}.actions{gap:2px}.actions button{padding:5px 6px}.test-status small{font-size:9px}.round-status{gap:8px}}
  @media(max-width:700px){:global(.ledger-dialog) footer{align-items:flex-start}:global(.ledger-dialog) footer span{max-width:55%}}
  @media(prefers-reduced-motion:reduce){.meta-section :global(.fold-icon),.criterion :global(.fold-icon){transition:none}}
</style>
