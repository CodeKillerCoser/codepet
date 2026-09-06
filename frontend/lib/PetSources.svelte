<script lang="ts">
  import { onMount } from 'svelte';
  import { petRequest, petSnapshot, type PetSource } from './petGateway';
  export let sources: PetSource[] = [];
  let version = 0;
  let error = '';
  let busy: string | null = null;
  const labels: Record<string, string> = { connecting: '正在连接', installed: '等待首次活动', receiving: '正在接收', error: '接入异常', disabled: '已关闭' };
  onMount(() => {
    let disposed = false; let loading = false;
    const refresh = async () => {
      if (loading || busy) return; loading = true; const currentVersion = version;
      try { const result = await petSnapshot(); if (!disposed && currentVersion === version) { sources = result.sources ?? []; error = ''; } }
      catch (e) { if (!disposed) error = String(e); }
      finally { loading = false; }
    };
    void refresh(); const timer = window.setInterval(() => void refresh(), 2000);
    return () => { disposed = true; window.clearInterval(timer); };
  });
  async function toggle(source: PetSource) {
    busy = source.id; version += 1;
    try { const result = await petRequest('source.setEnabled', { sourceId: source.id, enabled: !source.enabled }); sources = sources.map(s => s.id === source.id ? result.source : s); }
    catch (e) { error = String(e); }
    finally { busy = null; }
  }
</script>
<div class="agent-list">
  {#if error}<p role="alert">{error}</p>{/if}
  {#each sources as source (source.id)}
    <article class="agent-card source-card">
      <div class="agent-title"><span class="agent-kicker">{source.id}</span><h3>{source.displayName}</h3>
        <p class="agent-description">{source.message || '感知任务活动并展示在桌宠列表中'}</p>
      </div>
      <div class="agent-controls">
        <span class="status-chip" class:online={source.status === 'receiving'}>{labels[source.status] || source.status}</span>
        <button class="power-button" class:enabled={source.enabled} disabled={busy !== null} aria-pressed={source.enabled}
          aria-label={`${source.enabled ? '关闭' : '启用'} ${source.displayName} 活动感知`} on:click={() => toggle(source)}>{source.enabled ? '关闭' : '启用'}</button>
      </div>
    </article>
  {/each}
</div>

<style>
  .source-card { grid-template-areas: "title controls"; grid-template-columns: minmax(0, 1fr) auto; }
  .agent-description { overflow-wrap: anywhere; }
  @media (max-width: 840px) { .source-card { grid-template-areas: "title" "controls"; grid-template-columns: 1fr; } }
</style>
