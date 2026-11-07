<script lang="ts">
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
  $: pets = library?.pets ?? settings.petLibrary.pets;
  $: selected = pets.find(pet => pet.id === settings.pet.selectedPetId);
</script>

<section class="pet-editor pixel-panel">
  <header class="panel-head"><h3>桌宠形象</h3><span>{selected?.name ?? "橘猫"}</span></header>
  <div class="character-preview">
    <PetAvatar petId={settings.pet.selectedPetId} kind={settings.pet.kind} imagePath={settings.pet.imagePath} status={previewStatus} scale={4.5} label={selected?.name ?? "桌宠"} interactive />
    <p>点击抚摸，看看它的反应</p>
  </div>
  <div class="animation-picker" aria-label="预览宠物动作">
    {#each states as state}
      <button type="button" aria-pressed={previewStatus === state.status} on:click={() => previewStatus = state.status}>{state.label}</button>
    {/each}
  </div>
  <label class="size-control">
    <span>桌宠大小 <strong>{settings.pet.scale}×</strong></span>
    <input aria-label="桌宠大小" type="range" min="2" max="5" step="1" value={settings.pet.scale} on:change={(event) => onScale(Number(event.currentTarget.value))} />
  </label>
  <section class="pet-library-panel">
    <div class="panel-head compact"><h3>宠物库</h3></div>
    <div class="pet-list" aria-label="已配置宠物">
      {#each pets as pet (pet.id)}
        <article class="pet-item" class:locked={pet.id === "default" || pet.id === "builtin:panda"} class:active={settings.pet.selectedPetId === pet.id}>
          <button class="pet-select-button" type="button" aria-pressed={settings.pet.selectedPetId === pet.id} disabled={Boolean(busy)} on:click={() => onSelect(pet.id)}>
            <span class="pet-thumb"><PetAvatar petId={pet.id} kind={pet.kind} imagePath={pet.imagePath} animated={false} scale={1.1} label={pet.name} /></span>
            <span><strong>{pet.name}</strong><em>{pet.kind === "image" ? "图片角色" : pet.kind === "codex-atlas" ? "动画角色" : "内置角色"}</em></span>
            {#if settings.pet.selectedPetId === pet.id}<span aria-label="已选择">✓</span>{/if}
          </button>
          {#if pet.id !== "builtin:panda" && pet.id !== "default"}
            <button class="pet-delete-button" type="button" disabled={Boolean(busy)} aria-label={`删除 ${pet.name}`} on:click={(event) => onRemove(event, pet.id)}>×</button>
          {/if}
        </article>
      {/each}
    </div>
    <div class="import-actions">
      <button type="button" disabled={Boolean(busy)} on:click={onImportPackage}>{busy === "package" ? "正在导入…" : "导入动画包"}</button>
      <button type="button" disabled={Boolean(busy)} on:click={() => onImportImage(false)}>导入图片</button>
      <button type="button" disabled={Boolean(busy)} on:click={() => onImportImage(true)}>抠图导入</button>
    </div>
    <p class="import-hint">动画包请选择 pet.json；图片保留清晰度与透明背景。</p>
  </section>
</section>

<style>
  .pet-item.locked { grid-template-columns: minmax(0, 1fr); }
  .pet-editor { min-width: 0; }
  .character-preview { display: grid; justify-items: center; gap: 12px; padding: 24px 0 12px; border-radius: 16px; background: var(--app-surface-soft); }
  p { margin: 0; color: var(--color-text-secondary); font-size: var(--font-size-xs); }
  .animation-picker, .import-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
  .animation-picker button, .import-actions button { border: 1px solid var(--pet-border); border-radius: 8px; padding: 7px 10px; background: var(--pet-button-bg); color: var(--color-text-primary); cursor: pointer; font-size: var(--font-size-xs); }
  .animation-picker button[aria-pressed="true"] { background: var(--pet-active-bg); border-color: var(--pet-status-running); }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--pet-status-running); outline-offset: 3px; }
  button:disabled { opacity: 0.5; cursor: default; }
  .size-control { display: grid; gap: 8px; margin: 20px 0; font-size: var(--font-size-sm); }
  .size-control span { display: flex; justify-content: space-between; }
  .size-control input { width: 100%; }
  .import-hint { margin-top: 10px; }
</style>
