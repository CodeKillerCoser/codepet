<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import PetActivityGroups from '../PetActivityGroups.svelte';
  import PetAvatar from './PetAvatar.svelte';
  import { playWhipSound } from '../sound';
  import { defaultRunningBubbleSettings } from '../theme';
  import type { AppSettings, TaskStatus } from '../types';
  import type { PetTask, PetSource } from '../petGateway';
  import type { ActivityGroupId } from '../activityGroups';
  import type { SurfaceView } from './surface';
  export let settings: AppSettings | null;
  export let activities: PetTask[];
  export let sources: PetSource[];
  export let gatewayError: string;
  export let surface: SurfaceView;
  export let avatarStatus: TaskStatus;
  export let tasksCollapsed: boolean;
  export let actionNotice: string;
  export let expandedGroups: Set<ActivityGroupId>;
  export let dismissActivity: (event: MouseEvent, task: PetTask) => void;
  export let clearCompletedActivities: (event: MouseEvent) => void;
  export let dragWindow: () => Promise<void>;
  export let toggleSurface: (event: MouseEvent) => void;
  export let toggleTasks: (event: MouseEvent) => void;
  export let openMain: (event: MouseEvent) => void;
  let activityStack: HTMLElement | null = null;
  let lastTopActivityId: string | null = null;
  let whipAnimating = false;
  let whipAnimationKey = 0;
  let whipTimer: number | null = null;
  const whipVisibleMs = 760;
  $: hasActivities = activities.length > 0;
  $: hasCompletedActivities = activities.some(a => a.status === 'completed');
  $: runningBubble = settings?.appearance.runningBubble ?? defaultRunningBubbleSettings;
  $: activityStackHeightLimit = Math.max(0, Math.min(368, surface.availableHeight - 190));
  $: topActivityId = activities[0]?.id ?? null;
  $: if (activityStack && topActivityId !== lastTopActivityId) { lastTopActivityId = topActivityId; void tick().then(() => { if (activityStack) activityStack.scrollTop = 0; }); }
  onDestroy(clearWhipTimer);
  function clearWhipTimer() {
    if (whipTimer) {
      window.clearTimeout(whipTimer);
      whipTimer = null;
    }
  }

  function whipPet(event: MouseEvent) {
    event.stopPropagation();
    whipAnimationKey += 1;
    whipAnimating = true;
    clearWhipTimer();
    void playWhipSound(settings).catch((error) => {
      console.error("failed to play whip sound", error);
    });
    whipTimer = window.setTimeout(() => {
      whipAnimating = false;
      whipTimer = null;
    }, whipVisibleMs);
  }

</script>
  {#if !surface.dragging && !tasksCollapsed && (hasActivities || gatewayError)}
    <section class="activity-stack" bind:this={activityStack} aria-live="polite" style={`--pet-activity-stack-max-height: ${activityStackHeightLimit}px`}>
      {#if !hasActivities && gatewayError}
        <article class="status-pill gateway-state-pill"><div class="status-content">
          <span class="status-title">活动连接不可用</span>
          <span class="status-message">{gatewayError}</span>
        </div></article>
      {/if}
      <PetActivityGroups {activities} {sources} {runningBubble} bind:expanded={expandedGroups} dismiss={dismissActivity} />
    </section>
  {/if}
  <section class="pet-stage" aria-label="拖动移动桌宠">
    <div class="pet-action-rail">
      {#if surface.canDock}
        <button class="pet-action-button dock-button" type="button"
          aria-label={surface.physicalNotch ? '吸附到刘海' : '吸附到屏幕顶部'}
          title="点击停靠，也可以拖动角色到屏幕顶部中央"
          on:click={toggleSurface}><span aria-hidden="true">↑</span></button>
      {/if}
      {#if hasActivities}
        <button
          class="pet-action-button fold-button"
          class:collapsed={tasksCollapsed}
          type="button"
          aria-label={tasksCollapsed ? "展开任务列表" : "收起任务列表"}
          on:mousedown={(event) => event.stopPropagation()}
          on:click={toggleTasks}
        >
          <span aria-hidden="true"></span>
        </button>
      {/if}
      <button
        class="pet-action-button main-window-button"
        type="button"
        aria-label="打开主窗口"
        on:mousedown={(event) => event.stopPropagation()}
        on:click={openMain}
      >
        <span aria-hidden="true"></span>
      </button>
      <button
        class="pet-action-button whip-button"
        type="button"
        aria-label="抽鞭子"
        title="抽鞭子"
        on:mousedown={(event) => event.stopPropagation()}
        on:click={whipPet}
      >
        <span aria-hidden="true"></span>
      </button>
      {#if hasCompletedActivities}
        <button
          class="pet-action-button clear-completed-button"
          type="button"
          aria-label="移除全部已完成任务"
          title="移除全部已完成任务"
          on:mousedown={(event) => event.stopPropagation()}
          on:click={clearCompletedActivities}
        >
          <span aria-hidden="true"></span>
        </button>
      {/if}
    </div>
    {#key whipAnimationKey}
      <svg class="whip-animation whip-svg" class:active={whipAnimating} viewBox="0 0 460 340" aria-hidden="true">
        <defs>
          <linearGradient id="whipHandleGradient" x1="30" y1="296" x2="68" y2="263" gradientUnits="userSpaceOnUse">
            <stop offset="0" stop-color="var(--asset-whip-handle-gradient-start)" />
            <stop offset="0.45" stop-color="var(--asset-whip-handle-gradient-mid)" />
            <stop offset="1" stop-color="var(--asset-whip-handle-gradient-end)" />
          </linearGradient>
          <linearGradient id="whipFerruleGradient" x1="56" y1="255" x2="75" y2="274" gradientUnits="userSpaceOnUse">
            <stop offset="0" stop-color="var(--asset-whip-ferrule-gradient-start)" />
            <stop offset="0.45" stop-color="var(--asset-whip-ferrule-gradient-mid)" />
            <stop offset="1" stop-color="var(--asset-whip-ferrule-gradient-end)" />
          </linearGradient>
          <filter id="whipRopeTexture" x="-8%" y="-8%" width="116%" height="116%">
            <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="1" seed="8" result="grain" />
            <feColorMatrix in="grain" type="matrix" values="0 0 0 0 0.28 0 0 0 0 0.18 0 0 0 0 0.11 0 0 0 0.16 0" result="grainColor" />
            <feBlend in="SourceGraphic" in2="grainColor" mode="multiply" />
          </filter>
        </defs>
        <g class="whip-rig">
          <path class="motion-ghost" d="M 36 291 L 66 264 C 101 234, 132 207, 163 137 S 235 84, 286 96 C 306 101, 320 111, 332 126" pathLength="1" />
          <path class="handle-shadow" d="M 30 296 L 63 267" />
          <path class="handle-core" d="M 30 296 L 63 267" />
          <path class="handle-highlight" d="M 34 292 L 57 272" />
          <path class="handle-ring" d="M 35 289 L 43 298" />
          <path class="handle-ring" d="M 48 277 L 56 286" />
          <ellipse class="ferrule" cx="66" cy="263" rx="10" ry="7" transform="rotate(-43 66 263)" />
          <g class="join-knot">
            <ellipse class="knot-lobe" cx="76" cy="257" rx="12" ry="9" transform="rotate(-35 76 257)" />
            <ellipse class="knot-lobe" cx="84" cy="251" rx="8" ry="11" transform="rotate(32 84 251)" />
            <path class="knot-band" d="M 66 258 C 74 249, 82 247, 91 251" />
            <path class="knot-band" d="M 70 265 C 78 255, 87 254, 95 259" />
          </g>
          <path class="rope-shadow" d="M 86 254 C 112 230, 139 202, 164 138 S 238 85, 286 96 C 306 101, 320 111, 332 126" />
          <path class="rope-core rope-thick" d="M 86 254 C 112 230, 139 202, 164 138" />
          <path class="rope-core rope-mid" d="M 164 138 C 196 96, 241 85, 286 96" />
          <path class="rope-core rope-thin" d="M 286 96 C 306 101, 320 111, 332 126" />
          <path class="rope-strand strand-thick" d="M 86 254 C 112 230, 139 202, 164 138" />
          <path class="rope-strand light strand-thick" d="M 86 254 C 112 230, 139 202, 164 138" />
          <path class="rope-strand strand-mid" d="M 164 138 C 196 96, 241 85, 286 96" />
          <path class="rope-strand light strand-mid" d="M 164 138 C 196 96, 241 85, 286 96" />
          <path class="rope-strand strand-thin" d="M 286 96 C 306 101, 320 111, 332 126" />
          <path class="rope-strand light strand-thin" d="M 286 96 C 306 101, 320 111, 332 126" />
          <path class="tail-line" d="M 288 97 C 312 96, 336 104, 354 118" />
          <path class="tail-line" d="M 288 97 C 314 110, 333 128, 346 150" />
          <path class="tail-line" d="M 288 97 C 306 121, 316 143, 326 165" />
          <circle class="tail-knot" cx="354" cy="118" r="5" />
          <circle class="tail-knot" cx="346" cy="150" r="5" />
          <circle class="tail-knot" cx="326" cy="165" r="5" />
        </g>
        <g class="crack">
          <path d="M 332 126 L 302 111" />
          <path d="M 332 126 L 306 150" />
          <path d="M 332 126 L 369 118" />
          <path d="M 332 126 L 337 90" />
        </g>
      </svg>
    {/key}
    <PetAvatar
      petId={settings?.pet.selectedPetId ?? "default"}
      interactive
      onDrag={dragWindow}
      dragging={surface.dragging}
      startled={whipAnimating}
      kind={settings?.pet.kind}
      imagePath={settings?.pet.imagePath}
      status={avatarStatus}
      scale={Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4) * 0.9}
    />
    {#if surface.snapCandidate}
      <span class="pet-snap-hint" aria-live="polite">{surface.physicalNotch ? '松开吸附到刘海' : '松开吸附到顶部'}</span>
    {/if}
    {#if actionNotice}
      <span class="pet-notice">{actionNotice}</span>
    {/if}
  </section>

<style>
  .pet-stage { min-height: max(104px, var(--pet-rail-height)); }
  .pet-snap-hint { position: absolute; bottom: 0; white-space: nowrap; background: var(--pet-pill-bg); color: var(--pet-message); border-radius: 12px; padding: 3px 8px; font-size: 11px; pointer-events: none; }
  .dock-button span { font-size: 17px; line-height: 1; }
  .status-pill { cursor: default; }
</style>
