<script lang="ts">
  import { Check, Image as ImageIcon, Package, Plus, Sparkles } from "@lucide/svelte";
  import PetAvatar from "./PetAvatar.svelte";
  import type { AppSettings, PetLibraryView, TaskStatus } from "../types";

  export let settings: AppSettings;
  export let library: PetLibraryView | null = null;
  export let busy = "";
  export let onSelect: (id: string) => void;
  export let onRemove: (event: MouseEvent, id: string) => void;
  export let onImportImage: (cutout: boolean) => void;
  export let onImportPackage: () => void;
  export let onScale: (scale: number) => void;

  let previewStatus: TaskStatus = "idle";
  const states: { status: TaskStatus; label: string }[] = [
    { status: "idle", label: "待机" }, { status: "thinking", label: "思考" },
    { status: "running", label: "工作" }, { status: "waiting-approval", label: "等待" },
    { status: "done", label: "完成" }, { status: "failed", label: "出错" },
  ];
  const scaleTicks = [2, 3, 4, 5] as const;
  $: pets = library?.pets ?? settings.petLibrary.pets;
  $: selected = pets.find(pet => pet.id === settings.pet.selectedPetId);
  $: selectedKindLabel = selected?.kind === "image" ? "图片角色" : selected?.kind === "codex-atlas" ? "动画角色" : "内置角色";
  $: previewStateLabel = states.find(s => s.status === previewStatus)?.label ?? "待机";
</script>

<section class="pet-editor pixel-panel">
  <div class="pet-studio-hero">
    <div class="character-preview">
      <span class="stage-badge">{selected?.name ?? "橘猫"} · {selectedKindLabel}</span>
      <PetAvatar petId={settings.pet.selectedPetId} kind={settings.pet.kind} imagePath={settings.pet.imagePath} status={previewStatus} scale={4.5} label={selected?.name ?? "桌宠"} interactive />
      <p>点击抚摸，看看互动反应</p>
    </div>

    <div class="pet-studio-controls">
      <div class="control-block">
        <div class="control-head">
          <h3>动作预览</h3>
          <span>{previewStateLabel}</span>
        </div>
        <div class="animation-picker" aria-label="预览宠物动作">
          {#each states as state}
            <button type="button" aria-pressed={previewStatus === state.status} on:click={() => previewStatus = state.status}>{state.label}</button>
          {/each}
        </div>
      </div>

      <label class="size-control">
        <span class="control-head">
          <span>桌宠大小</span>
          <strong>{settings.pet.scale}×</strong>
        </span>
        <input aria-label="桌宠大小" type="range" min="2" max="5" step="1" value={settings.pet.scale} on:change={(event) => onScale(Number(event.currentTarget.value))} />
        <span class="scale-ticks" aria-hidden="true">
          {#each scaleTicks as tick}
            <span class:active={settings.pet.scale === tick}>{tick}×</span>
          {/each}
        </span>
      </label>
    </div>
  </div>

  <section class="pet-library-panel">
    <div class="library-head">
      <div class="library-title">
        <h3>宠物库</h3>
        <span>{pets.length} 个角色</span>
      </div>
      <div class="import-actions">
        <button type="button" disabled={Boolean(busy)} on:click={onImportPackage}><Package size={14} aria-hidden="true" />{busy === "package" ? "正在导入…" : "导入动画包"}</button>
        <button type="button" disabled={Boolean(busy)} on:click={() => onImportImage(false)}><ImageIcon size={14} aria-hidden="true" />导入图片</button>
        <button class="primary-action" type="button" disabled={Boolean(busy)} on:click={() => onImportImage(true)}><Sparkles size={14} aria-hidden="true" />抠图导入</button>
      </div>
    </div>

    <div class="pet-list" aria-label="已配置宠物">
      {#each pets as pet (pet.id)}
        <article class="pet-item" class:locked={pet.id === "default" || pet.id === "builtin:panda"} class:active={settings.pet.selectedPetId === pet.id}>
          <button class="pet-select-button" type="button" aria-pressed={settings.pet.selectedPetId === pet.id} disabled={Boolean(busy)} on:click={() => onSelect(pet.id)}>
            <span class="pet-thumb"><PetAvatar petId={pet.id} kind={pet.kind} imagePath={pet.imagePath} animated={false} scale={1.1} label={pet.name} /></span>
            <span class="pet-copy">
              <strong>{pet.name}</strong>
              <em>{pet.kind === "image" ? "图片角色" : pet.kind === "codex-atlas" ? "动画角色 · 6 种状态" : "内置角色 · 6 种状态"}</em>
            </span>
            {#if settings.pet.selectedPetId === pet.id}
              <span class="pet-active-check" aria-label="已选择"><Check size={12} aria-hidden="true" /></span>
            {/if}
          </button>
          {#if pet.id !== "builtin:panda" && pet.id !== "default"}
            <button class="pet-delete-button" type="button" disabled={Boolean(busy)} aria-label={`删除 ${pet.name}`} on:click={(event) => onRemove(event, pet.id)}>×</button>
          {/if}
        </article>
      {/each}
      <button class="pet-import-tile" type="button" disabled={Boolean(busy)} on:click={() => onImportImage(false)}>
        <Plus size={16} aria-hidden="true" />
        <span>导入图片或 pet.json</span>
      </button>
    </div>

    <p class="import-hint">动画包请选择 pet.json；静态图片自动保留清晰度与透明通道。</p>
  </section>
</section>

<style>
  .pet-editor { display: grid; gap: 24px; min-width: 0; }
  .pet-studio-hero {
    display: grid;
    grid-template-columns: minmax(0, 1.35fr) minmax(240px, 0.85fr);
    gap: 20px;
    padding: 18px;
    border: 1px solid var(--app-border);
    border-radius: 16px;
    background: var(--app-surface-soft);
  }
  .character-preview {
    position: relative;
    display: grid;
    min-height: 248px;
    place-items: center;
    align-content: center;
    gap: 12px;
    padding: 40px 16px 20px;
    border: 1px solid var(--color-main-outline, var(--app-border));
    border-radius: 12px;
    background-color: var(--app-surface);
    background-image: radial-gradient(var(--app-border) 1px, transparent 1px);
    background-size: 16px 16px;
  }
  .stage-badge {
    position: absolute;
    top: 12px;
    left: 12px;
    padding: 4px 10px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-soft);
    color: var(--app-muted);
    font-size: var(--font-size-caption);
    font-weight: var(--font-weight-medium);
  }
  .pet-studio-controls {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 20px;
    min-width: 0;
    padding: 4px 2px;
  }
  .control-block { display: grid; gap: 12px; }
  .control-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .control-head h3,
  .control-head > span:first-child {
    margin: 0;
    color: var(--app-text);
    font-size: var(--font-size-label);
    font-weight: var(--font-weight-semibold);
  }
  .control-head > span:last-child,
  .control-head strong {
    color: var(--app-muted);
    font-size: var(--font-size-caption);
    font-weight: var(--font-weight-medium);
    font-variant-numeric: tabular-nums;
  }
  .animation-picker {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }
  .animation-picker button {
    min-height: 36px;
    padding: 6px 10px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    color: var(--app-text);
    font-size: var(--font-size-label);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }
  .animation-picker button:hover { background: var(--color-main-hover, var(--app-surface-soft)); border-color: var(--app-border-strong); }
  .animation-picker button[aria-pressed="true"] {
    background: var(--color-main-selected, var(--pet-active-bg));
    border-color: var(--color-main-selected-border, var(--app-accent));
    color: var(--color-main-selected-text, var(--app-text));
  }
  .size-control { display: grid; gap: 10px; margin: 0; font-size: var(--font-size-label); }
  .size-control input { width: 100%; margin: 0; }
  .scale-ticks {
    display: flex;
    justify-content: space-between;
    color: var(--app-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }
  .scale-ticks .active { color: var(--app-text); font-weight: var(--font-weight-medium); }
  .pet-library-panel { display: grid; gap: 14px; min-width: 0; }
  .library-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
  }
  .library-title { display: flex; align-items: baseline; gap: 8px; }
  .library-title h3 { margin: 0; font: var(--font-section); color: var(--app-text); }
  .library-title span { color: var(--app-muted); font-size: var(--font-size-caption); }
  .import-actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .import-actions button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 32px;
    padding: 5px 12px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    color: var(--app-text);
    font-size: var(--font-size-caption);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }
  .import-actions button:hover:not(:disabled) { background: var(--color-main-hover, var(--app-surface-soft)); border-color: var(--app-border-strong); }
  .import-actions button.primary-action {
    background: var(--color-main-accent-solid);
    border-color: var(--color-main-accent-solid);
    color: var(--color-main-on-accent);
  }
  .import-actions button.primary-action:hover:not(:disabled) {
    background: var(--color-main-accent-hover);
    border-color: var(--color-main-accent-hover);
  }
  .pet-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(248px, 1fr));
    gap: 12px;
  }
  .pet-item {
    min-height: 76px;
    border-radius: 12px;
    background: var(--app-surface);
  }
  .pet-item.locked { grid-template-columns: minmax(0, 1fr); }
  .pet-select-button {
    grid-template-columns: 48px minmax(0, 1fr) auto;
    min-height: 74px;
    padding: 12px 14px;
    gap: 12px;
    border-radius: 12px;
    background: transparent;
  }
  .pet-select-button:hover { transform: none; }
  .pet-thumb {
    width: 48px;
    height: 48px;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface-soft);
  }
  .pet-copy { display: grid; gap: 3px; min-width: 0; text-align: left; }
  .pet-active-check {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 999px;
    background: var(--color-main-accent-solid);
    color: var(--color-main-on-accent);
  }
  .pet-import-tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 76px;
    padding: 12px 16px;
    border: 1px dashed var(--app-border-strong);
    border-radius: 12px;
    background: transparent;
    color: var(--app-muted);
    font-size: var(--font-size-caption);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }
  .pet-import-tile:hover:not(:disabled) {
    background: var(--app-surface-soft);
    color: var(--app-text);
    border-color: var(--app-border-strong);
  }
  p { margin: 0; color: var(--app-muted); font-size: var(--font-size-caption); }
  .import-hint { margin-top: 0; }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--color-focus-ring); outline-offset: 2px; }
  button:disabled { opacity: 0.5; cursor: default; }
  @media (max-width: 780px) {
    .pet-studio-hero { grid-template-columns: minmax(0, 1fr); }
  }
</style>
