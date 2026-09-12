<script lang="ts">
  import PetActivityGroups from '../lib/PetActivityGroups.svelte';
  import { defaultRunningBubbleSettings } from '../lib/theme';
  import type { PetTask } from '../lib/petGateway';
  let tasks: PetTask[] = ['waiting-input', 'waiting-approval', 'running', 'running', 'running', 'completed', 'completed'].map((status, index) => ({
    id: `${index}`, title: `任务 ${index}：检查桌宠气泡显示`, status: status as PetTask['status'],
    summary: index === 0 ? undefined : '**保留原来的消息预览**，只展开组内气泡。\n\n```ts\nconst longValue = "' + '长代码'.repeat(100) + '";\n```\n\n' + '更多详情不应自动展开。'.repeat(30),
    providerId: 'codex', updatedAt: 1789228800000,
  }));
  let animate = true;
  $: runningBubble = { ...defaultRunningBubbleSettings, backgroundColor: '#253545', borderColor: '#ff8833', borderWidth: 3, backgroundBreathing: animate, borderMarquee: animate };
</script>
<div class="pet-window main-theme light" style="width:100%;height:auto;min-height:100vh;justify-content:flex-start;padding-top:10px">
  <section class="activity-stack" style="--pet-activity-stack-max-height: 520px">
    <PetActivityGroups activities={tasks} sources={[{id:'codex',displayName:'Codex',enabled:true,status:'ready',message:''}]} {runningBubble}
      dismiss={(event, task) => { event.stopPropagation(); tasks = tasks.filter(t => t.id !== task.id); }} />
  </section>
  <button style="pointer-events:auto" on:click={() => animate = !animate}>切换动画</button>
</div>
