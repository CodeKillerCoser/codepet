<script lang="ts">
  import { onMount } from "svelte";
  import type { PetKind, TaskStatus } from "../types";
  import { actionFor, animations, atlas, frameAt, petSize, type PetAction, type PetInteraction } from "./animation";
  import { builtInPet, loadPetAsset } from "./assets";

  export let petId = "default";
  export let kind: PetKind | null | undefined = "builtin";
  export let imagePath: string | null | undefined = null;
  export let status: TaskStatus = "idle";
  export let scale = 3;
  export let width: number | undefined = undefined;
  export let label = "桌宠";
  export let interactive = false;
  export let animated = true;
  export let startled = false;
  export let dragging = false;
  export let portrait = false;
  export let onDrag: (() => Promise<void>) | undefined = undefined;

  let asset = { url: builtInPet(petId).url, atlas: true };
  let assetError = false;
  let mounted = false;
  let visible = true;
  let reducedMotion = false;
  let requestId = 0;
  let loadedKey = "";
  let timer: number | undefined;
  let interactionTimer: number | undefined;
  let interaction: PetInteraction = "none";
  let frame = frameAt("idle", 0);
  let press: { x: number; y: number; id: number; element: HTMLElement } | null = null;
  let dragged = false;

  $: size = width && Number.isFinite(width) && width > 0
    ? { width, height: Math.round(width * atlas.frameHeight / atlas.frameWidth) }
    : petSize(portrait ? 1.5 : scale);
  $: if (dragging && press) { dragged = true; releasePress(); }
  $: action = actionFor(status, dragging ? 'drag-right' : interaction, startled);
  $: emotion = startled || status === "failed" ? "!" : status === "waiting-approval" ? "?" : interaction === "pet" ? "♡" : status === "done" ? "✓" : "";
  $: assetKey = JSON.stringify([petId, kind, imagePath]);
  $: if (mounted && assetKey !== loadedKey) void refreshAsset(assetKey, petId, kind, imagePath);
  $: restart(action, asset.url, asset.atlas, animated, reducedMotion, visible, mounted);

  onMount(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncMotion = () => { reducedMotion = motion.matches; };
    const syncVisibility = () => { visible = !document.hidden; };
    syncMotion(); syncVisibility(); mounted = true;
    motion.addEventListener("change", syncMotion);
    document.addEventListener("visibilitychange", syncVisibility);
    return () => {
      mounted = false; requestId++;
      window.clearTimeout(timer); window.clearTimeout(interactionTimer);
      releasePress();
      motion.removeEventListener("change", syncMotion);
      document.removeEventListener("visibilitychange", syncVisibility);
    };
  });

  async function refreshAsset(key: string, id: string, type: PetKind | null | undefined, path?: string | null) {
    loadedKey = key;
    const request = ++requestId;
    assetError = false;
    interaction = "none";
    window.clearTimeout(interactionTimer);
    // Never leave the previous pet on screen while a new external file loads.
    asset = { url: builtInPet(id).url, atlas: true };
    try {
      const next = await loadPetAsset(id, type, path);
      if (request === requestId) asset = next;
    } catch {
      if (request === requestId) assetError = true;
    }
  }

  function recoverAsset() {
    assetError = true;
    if (asset.url !== builtInPet("default").url) asset = { url: builtInPet("default").url, atlas: true };
  }

  function validateImage(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    if (asset.atlas && (image.naturalWidth !== atlas.frameWidth * atlas.columns || image.naturalHeight !== atlas.frameHeight * atlas.rows)) recoverAsset();
  }

  function restart(nextAction: PetAction, _url: string, isAtlas: boolean, play: boolean, reduced: boolean, shown: boolean, ready: boolean) {
    window.clearTimeout(timer);
    frame = frameAt(nextAction, 0);
    if (!ready || !isAtlas || !play || reduced || !shown) return;
    const started = performance.now();
    const advance = () => {
      frame = frameAt(nextAction, performance.now() - started);
      timer = window.setTimeout(advance, frame.nextIn);
    };
    timer = window.setTimeout(advance, frame.nextIn);
  }

  function react(next: PetInteraction) {
    if (!interactive || press) return;
    window.clearTimeout(interactionTimer);
    interaction = next;
    const animation = animations[next === "pet" ? "celebrate" : "wave"];
    interactionTimer = window.setTimeout(() => { interaction = "none"; }, animation.durations.reduce((sum: number, duration) => sum + duration, 0));
  }

  function pointerDown(event: PointerEvent) {
    if (!interactive || event.button !== 0) return;
    dragged = false;
    const element = event.currentTarget as HTMLElement;
    press = { x: event.clientX, y: event.clientY, id: event.pointerId, element };
    element.setPointerCapture?.(event.pointerId);
  }

  async function pointerMove(event: PointerEvent) {
    if (!press || !onDrag || dragged || Math.hypot(event.clientX - press.x, event.clientY - press.y) < 5) return;
    const direction = event.clientX < press.x ? "drag-left" : "drag-right";
    dragged = true;
    releasePress();
    window.clearTimeout(interactionTimer);
    interaction = direction;
    try { await onDrag(); }
    finally { interaction = "none"; }
  }

  function releasePress() {
    const previous = press;
    press = null;
    if (previous?.element.hasPointerCapture?.(previous.id)) previous.element.releasePointerCapture(previous.id);
  }

  function clicked(event: MouseEvent) {
    if (!interactive || (dragged && event.detail !== 0)) return;
    react("pet");
  }
</script>

<svelte:element
  this={interactive ? "button" : "div"}
  type={interactive ? "button" : undefined}
  role={interactive ? undefined : "img"}
  class="pet-avatar"
  class:interactive
  class:portrait
  class:dragging={dragging || interaction === "drag-left" || interaction === "drag-right"}
  data-action={action}
  data-asset-error={assetError || undefined}
  aria-label={interactive ? `${label}，点击抚摸，拖动移动` : label}
  title={assetError ? "宠物素材无法加载，已显示默认角色" : undefined}
  style={`--pet-width: ${size.width}px; --pet-height: ${size.height}px;`}
  on:pointerdown={pointerDown}
  on:pointermove={pointerMove}
  on:pointerup={releasePress}
  on:pointercancel={releasePress}
  on:pointerenter={() => react("hover")}
  on:click={clicked}
>
  <span class="pet-frame" data-pet-frame data-columns={asset.atlas ? atlas.columns : 1} data-rows={asset.atlas ? atlas.rows : 1} data-column={asset.atlas ? frame.column : 0} data-row={asset.atlas ? frame.row : 0}>
    <img
      class:atlas={asset.atlas}
      src={asset.url}
      alt=""
      draggable="false"
      on:load={validateImage}
      on:error={recoverAsset}
      style={asset.atlas ? `width: ${size.width * atlas.columns}px; height: ${size.height * atlas.rows}px; transform: translate(${-frame.column * size.width}px, ${-frame.row * size.height}px);` : undefined}
    />
  </span>
  {#if emotion}<span class="pet-emotion" aria-hidden="true">{emotion}</span>{/if}
</svelte:element>

<style>
  .pet-avatar { position: relative; display: block; flex: none; width: var(--pet-width); height: var(--pet-height); padding: 0; border: 0; background: transparent; color: var(--pet-message); isolation: isolate; }
  .portrait { height: calc(var(--pet-width) * 2 / 3); overflow: hidden; }
  .portrait .pet-frame { width: var(--pet-width); height: var(--pet-height); left: 0; top: 0; filter: none; }
  .portrait .pet-emotion { display: none; }
  .interactive { cursor: grab; pointer-events: auto; touch-action: none; }
  .interactive.dragging { cursor: grabbing; }
  .interactive:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 4px; border-radius: 16px; }
  .pet-frame { position: absolute; inset: 0; display: block; overflow: hidden; pointer-events: none; filter: var(--pet-image-shadow-filter); }
  img { display: block; width: 100%; height: 100%; object-fit: contain; image-rendering: auto; pointer-events: none; user-select: none; }
  img.atlas { position: absolute; top: 0; left: 0; max-width: none; max-height: none; object-fit: fill; }
  .pet-emotion { position: absolute; right: 0; top: -6px; display: grid; place-items: center; min-width: 24px; height: 24px; border: 1px solid var(--pet-border); border-radius: 50%; background: var(--pet-pill-bg); box-shadow: 0 3px 8px var(--pet-shadow); font-size: var(--font-size-sm); font-weight: var(--font-weight-semibold); pointer-events: none; }
</style>
