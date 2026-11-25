// @vitest-environment jsdom
import {afterEach,beforeEach,expect,test,vi} from 'vitest';
import {mount,tick,unmount} from 'svelte';
import Timeline from './TaskRunTimeline.svelte';
import {taskLedgerFixture} from '../qa/taskLedgerFixture';
import {activityValue} from './taskActivity';
let component:ReturnType<typeof mount>;
beforeEach(()=>{vi.stubGlobal('ResizeObserver',class{observe(){}disconnect(){}});Object.defineProperty(document,'fonts',{configurable:true,value:{ready:Promise.resolve()}});HTMLElement.prototype.scrollIntoView=vi.fn();});
afterEach(async()=>{if(component)await unmount(component);document.body.replaceChildren();vi.unstubAllGlobals();vi.restoreAllMocks();});

test('comments, replies and structured changes are separate immutable activity entries',async()=>{
  const run=taskLedgerFixture().runs[0];
  run.timeline=[{id:'comment-a',at:run.updatedAt,actor:'claude',kind:'comment',summary:'<script>只显示评论正文</script>',agentConversation:[{agent:'claude',conversationId:'claude-chat-123',verified:false}]},
    {id:'state',at:run.updatedAt,actor:'cursor',kind:'status_changed',summary:'状态：开发中 → 待确认',changes:[{field:'status',before:'in_progress',after:'ready_for_delivery'}]},
    {id:'reply-b',at:run.updatedAt,actor:'cursor',kind:'comment',summary:'已经复核',replyTo:'comment-a'}];
  component=mount(Timeline,{target:document.body,props:{run}});await tick();
  expect(document.querySelectorAll('.activity-timeline>li')).toHaveLength(3);
  expect(document.querySelector('[title="claude-chat-123"]')).not.toBeNull();
  expect(document.querySelector('script')).toBeNull();
  const changes=document.querySelector('details')!;changes.open=true;
  expect(changes.textContent).toContain('开发中');expect(changes.textContent).toContain('待确认');
  (document.querySelector('.reply-link') as HTMLButtonElement).click();
  expect(document.activeElement?.id).toBe('activity-comment-a');
  expect(HTMLElement.prototype.scrollIntoView).toHaveBeenCalled();
  expect(document.querySelector('[aria-label*="编辑"]')).toBeNull();
});

test('old activity keeps its text and verification values retain the original plan labels',async()=>{
  const run=taskLedgerFixture().runs[0];
  run.timeline=[{id:'legacy',at:run.updatedAt,actor:'unknown',kind:'legacy',summary:'旧记录原样保留'}];
  component=mount(Timeline,{target:document.body,props:{run}});await tick();
  expect(document.body.textContent).toContain('历史 Agent');expect(document.body.textContent).toContain('旧记录原样保留');
  const criterion=run.validationPlan.items[0];
  expect(activityValue('verification',{planRevision:1,stage:'local',target:'snapshot',environment:'test',results:[{criterionId:criterion.id,result:'passed',evidence:'确实执行通过'}]},run)).toContain(criterion.title);
});
