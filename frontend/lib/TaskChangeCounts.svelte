<script lang="ts">
  import {changeKinds,type RunChanges,type FileKind} from './taskChanges';
  export let changes:RunChanges;
  export let loading=false;
  const order:FileKind[]=['added','modified','deleted','renamed','conflicted','unknown'];
</script>
<span class="change-counts" aria-label="变更文件分类">
  {#if loading}<small>读取变更…</small>{:else if !changes.complete}<small>变更信息不完整</small>{:else}
    {#each order as kind}{#if ['added','modified','deleted'].includes(kind)||changes.counts[kind]}
      <span class={`count ${kind}`} title={`${changeKinds[kind].label} ${changes.counts[kind]} 个文件`} aria-label={`${changeKinds[kind].label} ${changes.counts[kind]} 个文件`}><b>{changeKinds[kind].symbol}</b> {changes.counts[kind]}</span>
    {/if}{/each}
  {/if}
</span>
<style>
 .change-counts{display:inline-flex;gap:9px;align-items:center;flex-wrap:wrap;font:12px/1.5 var(--font-family-code);font-variant-numeric:tabular-nums}
 .count{white-space:nowrap;color:var(--wb-muted)} b{font-weight:500}
 .added{color:var(--color-success)}.modified,.renamed{color:var(--wb-info-text)}.deleted,.conflicted{color:var(--app-danger)}
 small{font:12px/1.5 var(--font-family-ui);color:var(--wb-muted)}
</style>
