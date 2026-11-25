<script lang="ts">
  import { onMount } from 'svelte';
  import { noticeList, noticeSourceSetEnabled, type NoticeSource } from './noticeGateway';
  export let sources: NoticeSource[] = [];
  let version = 0;
  let error = '';
  let busy: string | null = null;
  onMount(() => {
    let disposed = false; let loading = false;
    const refresh = async () => {
      if (loading || busy) return; loading = true; const currentVersion = version;
      try { const result = await noticeList(0,1); if (!disposed && currentVersion === version) { sources = result.sources ?? []; error = ''; } }
      catch (e) { if (!disposed) error = String(e); }
      finally { loading = false; }
    };
    void refresh(); const timer = window.setInterval(() => void refresh(), 2000);
    return () => { disposed = true; window.clearInterval(timer); };
  });
  async function toggle(source: NoticeSource) {
    busy = source.pluginId; version += 1;
    try { const result = await noticeSourceSetEnabled(source.pluginId,!source.enabled); sources = sources.map(s => s.pluginId === source.pluginId ? result.source : s); }
    catch (e) { error = String(e); }
    finally { busy = null; }
  }
</script>
<div class="agent-list">
  {#if error}<p role="alert">{error}</p>{/if}
  {#each sources as source (source.pluginId)}
    <article class="agent-card source-card">
      <div class="agent-title"><h3>{source.displayName}</h3>
        <p class="agent-description">{source.message || '接收通知并展示在桌宠列表中'}</p>
      </div>
      <div class="agent-controls">
        <span class="status-chip" class:online={source.connected}>{source.enabled ? source.connected ? '已连接' : '正在连接' : '已关闭'}</span>
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
