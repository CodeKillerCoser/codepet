// @vitest-environment jsdom
import { afterEach, expect, test, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import TaskKnowledgePage from './TaskKnowledgePage.svelte';
import { loadRecallGraph, renderRecallGraph } from './cogneeGraph';
vi.mock('./cogneeGraph', () => ({loadRecallGraph:vi.fn(),renderRecallGraph:vi.fn()}));
const graph = {nodes:[{id:'a'},{id:'b'}],links:[{source:'a',target:'b'}]};
let component: ReturnType<typeof mount> | undefined;
const settle = async () => { for (let i=0;i<6;i++) { await Promise.resolve(); await tick(); } };
afterEach(async () => { if (component) await unmount(component); component=undefined; document.body.innerHTML=''; vi.resetAllMocks(); });

test('uses a real snapshot, retains the view during busy refresh, and cleans it up',async () => {
  const destroy=vi.fn();
  vi.mocked(loadRecallGraph).mockResolvedValue(graph);
  vi.mocked(renderRecallGraph).mockResolvedValue({detail:{nodes:2,links:1},destroy});
  component=mount(TaskKnowledgePage,{target:document.body}); await settle();
  expect(document.body.textContent).not.toContain('个节点');
  expect(renderRecallGraph).toHaveBeenCalledWith(expect.any(HTMLElement),graph,expect.any(AbortSignal));
  let reject!: (cause:Error) => void;
  vi.mocked(loadRecallGraph).mockImplementationOnce(() => new Promise((_,fail) => {reject=fail;}));
  const refresh=(component as {refresh:()=>Promise<void>}).refresh(); await settle();
  expect(document.querySelector('[role=status]')?.textContent).toContain('更新图谱');
  expect(destroy).not.toHaveBeenCalled();
  reject(new Error('任务召回请求失败 (409)')); await settle();
  expect(document.querySelector('[role=status]')?.textContent).toContain('加工任务记录');
  expect(document.querySelector('[role=alert]')).toBeNull();
  await refresh;
  expect(document.querySelector('.graph-surface')).not.toBeNull();
  expect(destroy).not.toHaveBeenCalled();
  await unmount(component); component=undefined; expect(destroy).toHaveBeenCalledTimes(1);
});

test('empty graph directs the user to model settings without fabricating nodes',async () => {
  vi.mocked(loadRecallGraph).mockResolvedValue({nodes:[],links:[]});
  const onModels=vi.fn(); component=mount(TaskKnowledgePage,{target:document.body,props:{onModels}}); await settle();
  expect(document.body.textContent).toContain('还没有可预览的知识图谱');
  expect(renderRecallGraph).not.toHaveBeenCalled();
  [...document.querySelectorAll('button')].find(b=>b.textContent==='配置模型')!.click();
  expect(onModels).toHaveBeenCalledOnce(); expect(loadRecallGraph).toHaveBeenCalledOnce();
});

test('leaving before an IPC reply prevents a late renderer from being mounted',async () => {
  let finish!: (value:typeof graph) => void;
  vi.mocked(loadRecallGraph).mockImplementationOnce(() => new Promise(resolve=>{finish=resolve;}));
  component=mount(TaskKnowledgePage,{target:document.body}); await settle(); await unmount(component);component=undefined;
  finish(graph);await settle();expect(renderRecallGraph).not.toHaveBeenCalled();
});
