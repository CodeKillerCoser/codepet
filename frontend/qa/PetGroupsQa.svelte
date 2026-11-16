<script lang="ts">
  import NoticeGroups from '../lib/NoticeGroups.svelte';
  import { defaultRunningBubbleSettings } from '../lib/theme';
  import { notice,source } from '../lib/noticeTestFixtures';
  let tasks = ['waiting-input', 'waiting-approval', 'running', 'running', 'running', 'completed', 'completed'].map((status, index) => ({
    ...notice(`${index}`,1,status==='completed'), content:{title:`通知 ${index}：检查桌宠气泡显示`,progress:{text:'**保留消息预览**，只展开组内气泡。'+ '更多详情不应自动展开。'.repeat(30)}},
    actions:status.startsWith('waiting')?[{id:'reply',kind:'reply' as const,label:'回复',enabled:true}]:[],
  }));
  let animate = true;
  $: runningBubble = { ...defaultRunningBubbleSettings, backgroundColor: '#253545', borderColor: '#ff8833', borderWidth: 3, backgroundBreathing: animate, borderMarquee: animate };
</script>
<div class="pet-window main-theme light" style="width:100%;height:auto;min-height:100vh;justify-content:flex-start;padding-top:10px">
  <section class="activity-stack" style="--pet-activity-stack-max-height: 520px">
    <NoticeGroups activities={tasks} sources={[source]} {runningBubble}
      dismiss={(event, task) => { event.stopPropagation(); tasks = tasks.filter(t => t.noticeId !== task.noticeId); }} />
  </section>
  <button style="pointer-events:auto" on:click={() => animate = !animate}>切换动画</button>
</div>
