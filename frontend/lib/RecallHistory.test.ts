// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import Harness from '../qa/RecallHistoryHarness.svelte';
import { historyRequest, tokenLabel, type ProcessingJob } from './recallHistory';
vi.mock('./recallHistory', async importOriginal => ({...await importOriginal<typeof import('./recallHistory')>(), historyRequest:vi.fn()}));
const job: ProcessingJob = {
  id:'00000000-0000-4000-8000-000000000001',runId:'00000000-0000-4000-8000-000000000002',taskId:'00000000-0000-4000-8000-000000000003',
  title:'长任务的本轮加工',taskTitle:'任务主题',workspace:'/fixture',fingerprint:'test',contentHash:'test',state:'queued',
  queuedAt:1,startedAt:null,finishedAt:null,elapsedMs:null,phase:'',error:null,models:{llm:'model',embedding:'embedding'},
  resultSummary:'',usageTracked:true,canCancel:true,canRetry:false,
};
let component: ReturnType<typeof mount> | undefined;
const settle=async()=>{for(let i=0;i<15;i++){await Promise.resolve();await tick();}};
const button=(label:string)=>[...document.querySelectorAll('button')].find(b=>b.textContent===label)!;
beforeEach(()=>{
  vi.stubGlobal('matchMedia',vi.fn((query:string)=>({matches:query.includes('min-width') || query.includes('reduced-motion'),media:query,addEventListener:vi.fn(),removeEventListener:vi.fn()})));
  vi.stubGlobal('ResizeObserver',class {observe(){}unobserve(){}disconnect(){}});
  vi.mocked(historyRequest).mockImplementation(async(action)=> action==='detail' ? {...job,state:'completed',canCancel:false,result:{description:'原生结果',summaries:['框架原生摘要'],entities:[],indexUpdated:true}} : {items:[job],total:1,counts:{queued:1},offset:0,hasMore:false} as any);
});
afterEach(async()=>{if(component)await unmount(component);component=undefined;document.body.replaceChildren();vi.restoreAllMocks();vi.unstubAllGlobals();vi.resetAllMocks();});

test('shared global panel renders history and the pop-out opens a higher global dialog',async()=>{
  component=mount(Harness,{target:document.body});button('加工历史').click();await settle();
  const panel=document.querySelector('.app-side-panel')!;
  expect(panel.textContent).toContain(job.title);
  expect(panel.querySelector('.panel-card > .panel-header')?.textContent).toContain('加工历史');
  expect(panel.querySelector('.job-metadata')?.textContent).toContain('Token —');
  (panel.querySelector('[title="弹出加工详情"]') as HTMLButtonElement).click();await settle();
  const modal=document.querySelector('.recall-detail-dialog')!;
  expect(modal.textContent).toContain('框架原生摘要');
  expect(modal.closest('.app-side-panel')).toBeNull();
  expect(modal.closest('.app-window-host')).not.toBeNull();
  expect(document.querySelectorAll('.app-side-panel')).toHaveLength(1);
});

test('cancellation sends the attempt id and keeps the queue card on conflict',async()=>{
  component=mount(Harness,{target:document.body});button('加工历史').click();await settle();
  vi.mocked(historyRequest).mockImplementationOnce(async()=>{throw new Error('队列已变化');});
  button('取消排队').click();await settle();
  expect(historyRequest).toHaveBeenCalledWith('cancel',{jobId:job.id});
  expect(document.querySelector('[role=alert]')?.textContent).toContain('队列已变化');
  expect(document.querySelector('.history-job')?.textContent).toContain(job.title);
});

test('missing usage is never shown as free and partial usage stays explicit',()=>{
  const completed={...job,state:'completed' as const,usageTracked:false};
  expect(tokenLabel(completed)).toBe('Token 未报告');
  expect(tokenLabel({...completed,usageTracked:true,usage:{models:[],requests:2,reportedRequests:1,inputTokens:10,outputTokens:2,knownTokens:12,complete:false}})).toBe('12 Token（部分）');
});
