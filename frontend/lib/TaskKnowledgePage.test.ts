// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import TaskKnowledgePage from './TaskKnowledgePage.svelte';
import { loadRecallGraph, renderRecallGraph } from './cogneeGraph';
import { modelGatewayRequest, type ModelConfiguration } from './modelGateway';
import { recordPerfEvent } from './api';
vi.mock('./cogneeGraph', () => ({loadRecallGraph:vi.fn(),renderRecallGraph:vi.fn()}));
vi.mock('./modelGateway', () => ({modelGatewayRequest:vi.fn()}));
vi.mock('./api', () => ({recordPerfEvent:vi.fn().mockResolvedValue(undefined)}));
const configuration = (enabled = true): ModelConfiguration => ({settings:{enabled, llm:{endpoint:'http://127.0.0.1:11234/v1',model:'local-chat',apiKey:'',dimensions:0},embedding:{endpoint:'http://127.0.0.1:11234/v1',model:'local-embedding',apiKey:'',dimensions:1024},plainJson:false,llmExtraBody:{}},usage:{models:[]}});
beforeEach(() => { vi.mocked(modelGatewayRequest).mockResolvedValue(configuration()); vi.mocked(recordPerfEvent).mockResolvedValue(undefined); });
const graph = {nodes:[{id:'a'},{id:'b'}],links:[{source:'a',target:'b'}]};
let component: ReturnType<typeof mount> | undefined;
const settle = async () => { for (let i=0;i<6;i++) { await Promise.resolve(); await tick(); } };
afterEach(async () => { if (component) await unmount(component); component=undefined; document.body.innerHTML=''; vi.resetAllMocks(); vi.useRealTimers(); });

test('polls unchanged versions without redrawing and restores the current view on new versions',async () => {
  vi.useFakeTimers();
  const state={view:'schema',graph:{scale:2,x:30,y:40}};
  const oldDestroy=vi.fn(),nextDestroy=vi.fn();
  const first={...graph,previewVersion:'first',previewStatus:{updating:true,pending:14,processing:1,ready:1}};
  vi.mocked(loadRecallGraph).mockResolvedValueOnce(first).mockResolvedValueOnce({previewVersion:'first',unchanged:true}).mockResolvedValueOnce({...first,previewVersion:'second'});
  vi.mocked(renderRecallGraph).mockResolvedValueOnce({detail:{nodes:2,links:1},destroy:oldDestroy,getState:()=>state}).mockResolvedValueOnce({detail:{nodes:2,links:1},destroy:nextDestroy});
  component=mount(TaskKnowledgePage,{target:document.body}); await settle();
  await vi.advanceTimersByTimeAsync(5000); await settle();
  expect(loadRecallGraph).toHaveBeenLastCalledWith(false,'first');
  expect(renderRecallGraph).toHaveBeenCalledTimes(1);
  expect(oldDestroy).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(5000); await settle();
  expect(renderRecallGraph).toHaveBeenLastCalledWith(expect.any(HTMLElement),expect.objectContaining({previewVersion:'second'}),expect.any(AbortSignal),state);
  expect(oldDestroy).toHaveBeenCalledOnce();
  await unmount(component);component=undefined;
  await vi.advanceTimersByTimeAsync(10000);
  expect(loadRecallGraph).toHaveBeenCalledTimes(3);
  expect(nextDestroy).toHaveBeenCalledOnce();
});

test('the first completed preview appears automatically while the rest keeps processing',async () => {
  vi.useFakeTimers();
  vi.mocked(loadRecallGraph).mockRejectedValueOnce(new Error('(409)')).mockResolvedValue({...graph,previewVersion:'first',previewStatus:{updating:true,pending:13,processing:1,ready:1}});
  vi.mocked(renderRecallGraph).mockResolvedValue({detail:{nodes:2,links:1},destroy:vi.fn()});
  component=mount(TaskKnowledgePage,{target:document.body}); await settle();
  expect(document.body.textContent).toContain('正在准备首份图谱');
  await vi.advanceTimersByTimeAsync(5000); await settle();
  expect(renderRecallGraph).toHaveBeenCalledOnce();
  expect(document.body.textContent).not.toContain('正在准备首份图谱');
  expect(document.body.textContent).toContain('自动更新');
});

test('shows the published snapshot while connecting and clears startup status without redrawing',async () => {
  vi.useFakeTimers();
  vi.mocked(loadRecallGraph).mockResolvedValueOnce({...graph,previewVersion:'one',previewStatus:{preparing:true}})
    .mockResolvedValue({unchanged:true,previewVersion:'one',previewStatus:{updating:false}});
  vi.mocked(renderRecallGraph).mockResolvedValue({detail:{nodes:2,links:1},destroy:vi.fn()});
  component=mount(TaskKnowledgePage,{target:document.body});await settle();
  expect(document.body.textContent).toContain('先展示上次图谱');
  await vi.advanceTimersByTimeAsync(1000);await settle();
  expect(loadRecallGraph).toHaveBeenLastCalledWith(false,'one');
  expect(document.body.textContent).not.toContain('先展示上次图谱');
  expect(renderRecallGraph).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1000);expect(loadRecallGraph).toHaveBeenCalledTimes(2);
});

test('a user changing views during background rendering keeps the current renderer',async () => {
  vi.useFakeTimers();
  let state={view:'graph'};
  const destroy=vi.fn(),discard=vi.fn();
  vi.mocked(loadRecallGraph).mockResolvedValueOnce({...graph,previewVersion:'first'}).mockResolvedValue({...graph,previewVersion:'second'});
  let resolve!: (value:any)=>void;
  vi.mocked(renderRecallGraph).mockResolvedValueOnce({detail:{nodes:2,links:1},destroy,getState:()=>state})
    .mockImplementationOnce(()=>new Promise(done=>{resolve=done;}));
  component=mount(TaskKnowledgePage,{target:document.body});await settle();
  await vi.advanceTimersByTimeAsync(5000);await settle();
  state={view:'schema'};
  resolve({detail:{nodes:2,links:1},destroy:discard});await settle();
  expect(destroy).not.toHaveBeenCalled();expect(discard).toHaveBeenCalledOnce();
});

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

test('only missing models direct the empty graph to configuration',async () => {
  vi.mocked(loadRecallGraph).mockResolvedValue({nodes:[],links:[]});
  const missing = configuration(false); missing.settings.embedding.model = '';
  vi.mocked(modelGatewayRequest).mockResolvedValue(missing);
  const onModels=vi.fn(); component=mount(TaskKnowledgePage,{target:document.body,props:{onModels}}); await settle();
  expect(document.body.textContent).toContain('尚未配置任务模型');
  expect(renderRecallGraph).not.toHaveBeenCalled();
  [...document.querySelectorAll('button')].find(b=>b.textContent==='配置模型')!.click();
  expect(onModels).toHaveBeenCalledOnce(); expect(loadRecallGraph).toHaveBeenCalledOnce();
});

test('configured local models with processing disabled do not ask for reconfiguration',async () => {
  vi.mocked(loadRecallGraph).mockResolvedValue({nodes:[],links:[]});
  vi.mocked(modelGatewayRequest).mockResolvedValue(configuration(false));
  const onModels=vi.fn(); component=mount(TaskKnowledgePage,{target:document.body,props:{onModels}}); await settle();
  expect(document.body.textContent).toContain('任务加工尚未启用');
  expect(document.body.textContent).toContain('模型已配置');
  expect(document.body.textContent).not.toContain('配置模型');
  [...document.querySelectorAll('button')].find(b=>b.textContent==='模型设置')!.click();
  expect(onModels).toHaveBeenCalledOnce();
  expect(modelGatewayRequest).toHaveBeenCalledWith('model-settings');
  // The empty state never changes the user's processing setting itself.
  expect(modelGatewayRequest).toHaveBeenCalledOnce();
});

test('enabled empty index offers refresh and rechecks settings on refresh',async () => {
  vi.mocked(loadRecallGraph).mockResolvedValue({nodes:[],links:[]});
  component=mount(TaskKnowledgePage,{target:document.body}); await settle();
  expect(document.body.textContent).toContain('任务加工已启用');
  expect(document.body.textContent).not.toContain('配置模型');
  vi.mocked(modelGatewayRequest).mockResolvedValue(configuration(false));
  [...document.querySelectorAll('button')].find(b=>b.textContent==='刷新图谱')!.click(); await settle();
  expect(document.body.textContent).toContain('任务加工尚未启用');
  expect(modelGatewayRequest).toHaveBeenCalledTimes(2);
});

test('unreadable settings are an error rather than a missing-model claim',async () => {
  vi.mocked(loadRecallGraph).mockResolvedValue({nodes:[],links:[]});
  vi.mocked(modelGatewayRequest).mockRejectedValue(new Error('模型配置文件无效'));
  component=mount(TaskKnowledgePage,{target:document.body}); await settle();
  expect(document.querySelector('[role=alert]')?.textContent).toContain('模型配置文件无效');
  expect(document.body.textContent).not.toContain('尚未配置任务模型');
  expect(document.body.textContent).not.toContain('配置模型');
});

test('leaving before an IPC reply prevents a late renderer from being mounted',async () => {
  let finish!: (value:typeof graph) => void;
  vi.mocked(loadRecallGraph).mockImplementationOnce(() => new Promise(resolve=>{finish=resolve;}));
  component=mount(TaskKnowledgePage,{target:document.body}); await settle(); await unmount(component);component=undefined;
  finish(graph);await settle();expect(renderRecallGraph).not.toHaveBeenCalled();
});
