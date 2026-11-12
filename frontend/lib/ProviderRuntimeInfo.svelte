<script lang="ts">
  import { Plug, ChevronDown, ShieldAlert } from "@lucide/svelte";
  import type { AgentRuntime } from "./types";
  import type { ProviderConnectionState } from "./providerRuntimes";
  import ProviderConnectionStatus from "./ProviderConnectionStatus.svelte";

  export let runtime: AgentRuntime;
  export let connection: ProviderConnectionState | undefined;
  export let connectionsLoaded = false;
  export let busy = false;
  export let onSelect: (runtime: AgentRuntime, path: string) => void;
  let failedIcon: string | null = null;
  $: name = connection?.displayName || runtime.displayName;
</script>

<article class="provider-info" aria-label={name}>
  <header>
    <div class="provider-icon" aria-hidden="true">
      {#if connection?.icon && failedIcon !== connection.icon}
        <img src={connection.icon} alt="" referrerpolicy="no-referrer" on:error={() => failedIcon = connection?.icon ?? null} />
      {:else}<Plug size={24} />{/if}
    </div>
    <div class="provider-identity">
      <h3>{name}</h3>
      <ProviderConnectionStatus current={connection} available={connectionsLoaded} />
    </div>
  </header>
  <dl>
    {#if runtime.status === "not-required"}
      {#if connection?.version}<div><dt>Provider 版本</dt><dd>{connection.version}</dd></div>{/if}
      <div><dt>运行时</dt><dd>由 Provider 管理，无需选择</dd></div>
    {:else}
    {#if connection?.version}<div><dt>Provider 版本</dt><dd>{connection.version}</dd></div>{/if}
    <div>
      <dt><label for={`runtime-path-${runtime.providerId}`}>运行时路径</label></dt>
      <dd>
        <div class="path-select">
          <select id={`runtime-path-${runtime.providerId}`} title={runtime.resolvedExecutable ?? ""} value={runtime.resolvedExecutable ?? ""}
            disabled={busy || runtime.status === "loading" || !runtime.installed?.length}
            on:change={(event) => onSelect(runtime, event.currentTarget.value)}>
            {#if runtime.resolvedExecutable && !runtime.installed?.some((installation) => installation.executablePath === runtime.resolvedExecutable)}<option value={runtime.resolvedExecutable}>{runtime.resolvedExecutable}</option>{/if}
            {#if !runtime.resolvedExecutable}<option value="">{runtime.status === "loading" ? "检测中…" : "未选择运行时"}</option>{/if}
            {#each runtime.installed ?? [] as installation}
              <option value={installation.executablePath} disabled={!!installation.incompatibilityReason}>{installation.executablePath}{installation.incompatibilityReason ? `（${installation.incompatibilityReason}）` : ""}</option>
            {/each}
          </select>
          <ChevronDown size={14} aria-hidden="true" />
        </div>
      </dd>
    </div>
    <div><dt>运行时版本</dt><dd>{runtime.version ?? "—"}</dd></div>
    {/if}
    {#if connection?.defaultWorkspaceRoot}<div><dt>默认工作目录</dt><dd class="path-value">{connection.defaultWorkspaceRoot}</dd></div>{/if}
  </dl>
  {#if runtime.diagnostic}
    <div class="diagnostic" role="status"><ShieldAlert size={16} /><p>{runtime.diagnostic.message}</p></div>
  {/if}
</article>

<style>
  .provider-info { display: grid; align-content: start; gap: 16px; min-width: 0; padding: 20px 0; }
  .provider-info + .provider-info { border-top: 1px solid var(--app-border); }
  header { display: flex; align-items: center; gap: 12px; }
  .provider-icon { width: 40px; height: 40px; flex: 0 0 40px; display: grid; place-items: center; color: var(--app-muted); }
  .provider-icon img { width: 32px; height: 32px; object-fit: contain; }
  .provider-identity { min-width: 0; display: grid; gap: 5px; }
  h3 { margin: 0; font-size: var(--font-size-md); overflow-wrap: anywhere; }
  .provider-identity :global(.provider-connection) { margin: 0; }
  dl { display: grid; gap: 2px; margin: 0; }
  dl > div { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: 20px; align-items: center; min-height: 32px; }
  dt, dd { margin: 0; min-width: 0; font-size: var(--font-size-sm-plus); }
  dt { color: var(--app-muted); }
  dd { text-align: right; overflow-wrap: anywhere; }
  .path-value { font-family: var(--font-family-mono, monospace); font-size: var(--font-size-sm); }
  .path-select { position: relative; min-width: 0; }
  select { appearance: none; width: 100%; min-width: 0; border: 1px solid var(--app-border); border-radius: 6px; background: var(--app-surface); color: var(--app-text); font: inherit; font-size: var(--font-size-sm); padding: 7px 28px 7px 10px; text-overflow: ellipsis; }
  select:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  select:disabled { color: var(--app-muted); }
  .path-select :global(svg) { position: absolute; right: 9px; top: 50%; transform: translateY(-50%); pointer-events: none; color: var(--app-muted); }
  .diagnostic { display: flex; align-items: start; gap: 8px; padding: 10px; border-radius: 6px; color: var(--app-danger); background: color-mix(in srgb, var(--app-danger) 6%, var(--app-surface)); }
  .diagnostic p { margin: 0; overflow-wrap: anywhere; font-size: var(--font-size-sm); }
  .diagnostic :global(svg) { flex-shrink: 0; }
  @media (max-width: 640px) {
    dl > div { grid-template-columns: 90px minmax(0, 1fr); gap: 12px; }
  }
</style>
