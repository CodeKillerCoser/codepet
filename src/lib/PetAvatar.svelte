<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { atlasAnimationForStatus } from "./petAtlas";
  import PetSprite from "./PetSprite.svelte";
  import type { PetKind, PixelPetSprite, TaskStatus } from "./types";

  export let sprite: PixelPetSprite;
  export let kind: PetKind | null | undefined = "palette";
  export let imagePath: string | null | undefined = null;
  export let status: TaskStatus = "idle";
  export let scale = 3;
  export let label = "pet";

  let atlasFrame = 0;
  let atlasTimer: number | null = null;
  let atlasAnimationKey = "";
  let atlasDataUrl = "";
  let atlasDataPath = "";

  $: safeScale = Math.min(Math.max(scale, 2), 5);
  $: imageUrl = imagePath ? convertFileSrc(imagePath) : "";
  $: atlasUrl = atlasDataUrl || imageUrl;
  $: statusClass = status === "waiting-approval" || status === "failed" ? "alerting" : status;
  $: if (kind === "codex-atlas" && imagePath && imagePath !== atlasDataPath) {
    void loadAtlasDataUrl(imagePath);
  }
  $: if (kind !== "codex-atlas" || !imagePath) {
    atlasDataUrl = "";
    atlasDataPath = "";
  }
  $: isCodexAtlas = kind === "codex-atlas" && Boolean(atlasUrl);
  $: atlasAnimation = atlasAnimationForStatus(status);
  $: atlasCellWidth = Math.round(32 * safeScale);
  $: atlasCellHeight = Math.round(atlasCellWidth * (208 / 192));
  $: atlasColumn = atlasAnimation.frames[atlasFrame]?.column ?? 0;
  $: atlasStyle = [
    `width: ${atlasCellWidth}px`,
    `height: ${atlasCellHeight}px`,
  ].join("; ");
  $: atlasImageStyle = [
    `width: ${atlasCellWidth * 8}px`,
    `height: ${atlasCellHeight * 9}px`,
    `transform: translate(${-atlasColumn * atlasCellWidth}px, ${-atlasAnimation.row * atlasCellHeight}px)`,
  ].join("; ");
  $: nextAtlasAnimationKey = `${atlasUrl}:${status}`;
  $: if (isCodexAtlas && nextAtlasAnimationKey !== atlasAnimationKey) {
    restartAtlasAnimation(nextAtlasAnimationKey);
  }
  $: if (!isCodexAtlas && atlasAnimationKey) {
    stopAtlasAnimation();
  }

  onDestroy(stopAtlasAnimation);

  async function loadAtlasDataUrl(path: string) {
    atlasDataPath = path;
    try {
      const dataUrl = await invoke<string>("pet_asset_data_url", { path });
      if (atlasDataPath === path) {
        atlasDataUrl = dataUrl;
      }
    } catch {
      if (atlasDataPath === path) {
        atlasDataUrl = "";
      }
    }
  }

  function restartAtlasAnimation(key: string) {
    stopAtlasAnimation();
    atlasAnimationKey = key;
    atlasFrame = 0;
    scheduleNextAtlasFrame();
  }

  function scheduleNextAtlasFrame() {
    const frame = atlasAnimation.frames[atlasFrame] ?? atlasAnimation.frames[0];
    atlasTimer = window.setTimeout(() => {
      atlasFrame = (atlasFrame + 1) % atlasAnimation.frames.length;
      scheduleNextAtlasFrame();
    }, frame.durationMs);
  }

  function stopAtlasAnimation() {
    if (atlasTimer) {
      window.clearTimeout(atlasTimer);
      atlasTimer = null;
    }
    atlasAnimationKey = "";
    atlasFrame = 0;
  }
</script>

{#if isCodexAtlas}
  <div class={`pet-atlas ${statusClass}`} style={atlasStyle} role="img" aria-label={label}>
    <img src={atlasUrl} alt="" style={atlasImageStyle} draggable="false" />
  </div>
{:else if imageUrl}
  <img
    class={`pet-image ${statusClass}`}
    src={imageUrl}
    alt={label}
    style={`--pet-image-size: ${Math.round(28 * safeScale)}px`}
    draggable="false"
  />
{:else}
  <PetSprite {sprite} {status} scale={safeScale} />
{/if}
