<script lang="ts">
  import PathLabel from "./PathLabel.svelte";
  import { providerKindsFromFeatures, providerKindLabel, type ProviderKind } from "./providerPresentation";
  import { Plug, ChevronDown, ShieldAlert, Trash2 } from "@lucide/svelte";
  import type { AgentRuntime } from "./types";
  import type { ProviderConnectionState } from "./providerRuntimes";
  import ProviderConnectionStatus from "./ProviderConnectionStatus.svelte";

  export let runtime: AgentRuntime;
  export let connection: ProviderConnectionState | undefined;
  export let connectionsLoaded = false;
  export let busy = false;
  export let onSelect: (runtime: AgentRuntime, path: string) => void;
  export let onRemove: ((runtime: AgentRuntime) => void) | undefined = undefined;
  export let kind: ProviderKind | undefined = undefined;
  let displayKind: ProviderKind | undefined;
  $: displayKinds = kind ? [kind] : providerKindsFromFeatures(connection?.features);
  $: displayKind = displayKinds.includes('agent') ? 'agent' : displayKinds.includes('observation') ? 'observation' : undefined;
  $: showRuntime = connection?.features?.includes('runtimeInventory') ?? false;
  $: observerOnly = displayKind === 'observation' && !displayKinds.includes('agent');
  $: runtimePath = runtime.resolvedExecutable || runtime.configuredExecutable || (runtime.status === 'loading' ? '检测中…' : observerOnly ? '不需要' : '未提供');
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
      <div class="name-line"><h3 title={name}>{name}</h3>{#each displayKinds as feature}<span class="kind">{providerKindLabel(feature)}</span>{/each}</div>
      <ProviderConnectionStatus current={connection} available={connectionsLoaded} kind={displayKind} compact />
    </div>
  </header>
  <div class="provider-details" class:removable={!!onRemove}>
  <dl>
    <div><dt>插件版本</dt><dd>{connection?.version || '未提供'}</dd></div>
    <div>
      <dt>{#if showRuntime && runtime.status !== "not-required"}<label for={`runtime-path-${runtime.providerId}`}>运行时路径</label>{:else}运行时路径{/if}</dt>
      <dd>
        {#if showRuntime && runtime.status !== "not-required"}
        <div class="path-select">
          <div class="path-display" aria-hidden="true"><PathLabel value={runtimePath} /></div>
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
        {:else}<PathLabel value={runtimePath} />{/if}
      </dd>
    </div>
    <div><dt>运行时版本</dt><dd>{runtime.version || (observerOnly ? '不需要' : '未提供')}</dd></div>
    <div><dt>默认工作目录</dt><dd class="path-value"><PathLabel value={connection?.defaultWorkspaceRoot || '未提供'} /></dd></div>
  </dl>
  {#if onRemove}<button type="button" class="remove-provider" disabled={busy} aria-label={`移除 ${name}`} title={`移除 ${name}`} on:click={() => onRemove?.(runtime)}><Trash2 size={16} aria-hidden="true" /></button>{/if}
  </div>
  {#if runtime.diagnostic}
    <div class="diagnostic" role="status"><ShieldAlert size={16} /><p>{runtime.diagnostic.message}</p></div>
  {/if}
</article>

<style>
  .provider-info { display: grid; align-content: start; box-sizing: border-box; min-height: 220px; grid-template-columns: minmax(0, 1fr); overflow-x: hidden; overflow-y: auto; gap: 14px; min-width: 0; padding: 18px; border: 1px solid var(--app-border); border-radius: 12px; background: var(--app-surface); }
  header { display: flex; align-items: center; gap: 12px; }
  .provider-icon { width: 40px; height: 40px; flex: 0 0 40px; display: grid; place-items: center; color: var(--app-muted); }
  .provider-icon img { width: 32px; height: 32px; object-fit: contain; }
  .provider-details { position: relative; min-width: 0; }
  .provider-details.removable dl > div:last-child dd { padding-right: 34px; }
  .remove-provider { position: absolute; right: 0; bottom: 2px; display: grid; place-items: center; width: 28px; height: 28px; color: var(--app-danger); background: transparent; border: 0; border-radius: 5px; padding: 0; cursor: pointer; }
  .remove-provider:hover { background: color-mix(in srgb, var(--app-danger) 8%, var(--app-surface)); }
  .remove-provider:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  .remove-provider:disabled { opacity: .5; cursor: default; }
  .provider-identity { flex: 1; min-width: 0; display: grid; gap: 5px; }
  .name-line { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .kind { font-size: var(--font-size-xs); color: var(--app-muted); padding: 2px 6px; border-radius: 4px; background: var(--app-surface-soft); white-space: nowrap; flex-shrink: 0; }
  h3 { margin: 0; min-width: 0; font: var(--font-subheading); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .provider-identity :global(.provider-connection) { margin: 0; }
  dl { display: grid; gap: 2px; margin: 0; }
  dl > div { display: grid; grid-template-columns: 90px minmax(0, 1fr); gap: 12px; align-items: center; min-height: 32px; }
  dt, dd { margin: 0; min-width: 0; font-size: var(--font-size-sm); }
  dt { color: var(--app-muted); }
  dd { text-align: right; overflow-wrap: anywhere; }
  .path-value { font-family: var(--font-family-mono, monospace); font-size: var(--font-size-sm); }
  .path-select { position: relative; min-width: 0; height: 32px; border-radius: 5px; }
  .path-display { position: absolute; inset: 0 24px 0 0; display: grid; align-items: center; min-width: 0; pointer-events: none; }
  select { appearance: none; position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; }
  select:disabled { cursor: default; }
  .path-select:focus-within { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  .path-select:hover { background: var(--app-surface-soft); }
  .path-select :global(svg) { position: absolute; right: 2px; top: 50%; transform: translateY(-50%); pointer-events: none; color: var(--app-muted); }
  .diagnostic { display: flex; align-items: start; gap: 8px; padding: 10px; border-radius: 6px; color: var(--app-danger); background: color-mix(in srgb, var(--app-danger) 6%, var(--app-surface)); }
  .diagnostic p { margin: 0; overflow-wrap: anywhere; font-size: var(--font-size-sm); }
  .diagnostic :global(svg) { flex-shrink: 0; }
  @media (max-width: 640px) {
    dl > div { grid-template-columns: 90px minmax(0, 1fr); gap: 12px; }
  }
</style>
