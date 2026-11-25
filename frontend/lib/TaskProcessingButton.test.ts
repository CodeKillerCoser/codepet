// @vitest-environment jsdom
import {afterEach, expect, test, vi} from 'vitest';
import {mount, tick, unmount} from 'svelte';
import TaskCollection from './TaskCollection.svelte';
import {historyRequest} from './recallHistory';
import {taskLedgerFixture} from '../qa/taskLedgerFixture';
vi.mock('./recallHistory',()=>({historyRequest:vi.fn()}));
let component:ReturnType<typeof mount>;
const settle=async()=>{for(let i=0;i<6;i++){await Promise.resolve();await tick();}};
afterEach(async()=>{if(component)await unmount(component);document.body.replaceChildren();vi.resetAllMocks();});

test.each(['board','table'] as const)('%s processing queues the task without opening it and blocks duplicate clicks',async view=>{
  const task=taskLedgerFixture(),onSelect=vi.fn(),feedback=vi.fn();
  let resolve:(value:unknown)=>void=()=>{};
  vi.mocked(historyRequest).mockImplementation(()=>new Promise(done=>resolve=done));
  component=mount(TaskCollection,{target:document.body,props:{tasks:[task],total:1,view,onSelect,onSort:vi.fn(),onProcessingFeedback:feedback}});
  const button=document.querySelector<HTMLButtonElement>('.process-task')!;
  expect(button.closest('button')?.parentElement?.closest('button')).toBeNull();
  button.click();button.click();await settle();
  expect(historyRequest).toHaveBeenCalledTimes(1);
  expect(historyRequest).toHaveBeenCalledWith('enqueue',{taskId:task.id,operationId:expect.any(String)});
  expect(button.disabled).toBe(true);expect(onSelect).not.toHaveBeenCalled();
  resolve({queued:1,existing:1});await settle();
  expect(feedback).toHaveBeenLastCalledWith(expect.stringContaining('1 次执行已加入加工队列'),false);
  expect(button.disabled).toBe(false);
});

test('ambiguous failures reuse the operation id and developing-only tasks cannot queue',async()=>{
  const task=taskLedgerFixture(),feedback=vi.fn();
  vi.mocked(historyRequest).mockRejectedValueOnce(Error('连接中断')).mockResolvedValue({queued:1,existing:0});
  component=mount(TaskCollection,{target:document.body,props:{tasks:[task],view:'table',onSelect:vi.fn(),onSort:vi.fn(),onProcessingFeedback:feedback}});
  const button=document.querySelector<HTMLButtonElement>('.process-task')!;
  button.click();await settle();expect(feedback).toHaveBeenLastCalledWith('连接中断',true);
  button.click();await settle();
  expect(vi.mocked(historyRequest).mock.calls[0][1]).toEqual(vi.mocked(historyRequest).mock.calls[1][1]);
  await unmount(component);
  task.runs.forEach(run=>run.status='in_progress');
  component=mount(TaskCollection,{target:document.body,props:{tasks:[task],view:'table',onSelect:vi.fn(),onSort:vi.fn()}});
  expect(document.querySelector<HTMLButtonElement>('.process-task')!.disabled).toBe(true);
});
