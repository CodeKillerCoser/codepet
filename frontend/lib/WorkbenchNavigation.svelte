<script lang="ts">
  import HorizontalTabs from './HorizontalTabs.svelte';
  export let view: 'code' | 'tasks' = 'code';
  const items = [{ route: 'code', label: '代码状态' }, { route: 'tasks', label: '任务与验收' }] as const;
</script>

<HorizontalTabs {items} value={view} label="工作台视图" panelId="codex-workbench-panel" onChange={next => view = next} />
