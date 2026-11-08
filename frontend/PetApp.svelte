<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';
  import { getAppSettings, openMainWindow } from './lib/api';
  import DesktopPet from './lib/pet/DesktopPet.svelte';
  import NotchPet from './lib/pet/NotchPet.svelte';
  import type { ProviderConnectionState } from './lib/providerRuntimes';
  import { createPetData } from './lib/pet/model';
  import { initialPresentation, present, type PresentationEvent } from './lib/pet/presentation';
  import type { ActivityGroupId } from './lib/activityGroups';

  import { type PetTask } from './lib/petGateway';

  import { rectFromElementBounds, shouldIgnorePetWindowCursor, type PetHitRect } from './lib/petHitTest';
  import { isPointOnOpaquePetImage } from './lib/pet/hitTest';
  import { taskStatusForPet } from './lib/pet/animation';
  import { PetSurfaceController } from './lib/pet/surface';
  import { PetMenuController } from './lib/pet/menu';
  import { petSizing } from './lib/pet/sizing';
  import { themeClassNames } from './lib/theme';
  import type { AppSettings } from './lib/types';
  const menuController = new PetMenuController();
  let settings: AppSettings | null = null;
  let presentation = initialPresentation;
  const petData = createPetData(() => dispatch({ type: 'new-task' }));
  let expandedGroups = new Set<ActivityGroupId>();
  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  let systemDark = false;
  let ready = false;
  let actionNotice = '';
  let noticeTimer: number | null = null;
  $: surface = presentation.surface;
  let surfaceController: PetSurfaceController | null = null;
  let petWindowElement: HTMLElement | null = null;
  let petContentElement: HTMLElement | null = null;
  let cursorPassthroughTimer: number | null = null;
  let cursorEventsIgnored = false;
  let petDragging = false;
  let cursorPassthroughInFlight = false;
  const noticeVisibleMs = 2500;
  const cursorPassthroughPollMs = 60;
  const petHitPadding = 2;
  const devMode = import.meta.env.DEV;
  const defaultPetOpacity = 1;
  const minPetOpacity = 0.25;
  $: themeClass = themeClassNames(settings?.appearance.theme === 'dark' || (settings?.appearance.theme === 'system' && systemDark) ? 'dark' : 'light');
  $: activities = $petData.tasks;
  $: avatarStatus = taskStatusForPet(activities);
  $: petWindowOpacity = clampPetOpacity(settings?.pet.opacity);
  $: petDragging = surface.dragging;
  $: railHeight = (2 + Number(activities.length > 0) + Number(activities.some(a => a.status === 'completed')) + Number(surface.canDock)) * 28;
  $: sizing = petSizing(surface, settings?.pet.scale, railHeight);
  $: if (ready) surfaceController?.schedule();
  $: if (ready && surface.menuBar && settings) void menuController.update(settings, activities, $petData.error).catch(error => console.warn('Pet menu update failed', error));
  function dispatch(event: PresentationEvent) {
    if (surface.menuBar && surface.mode === 'docked' && event.type === 'collapse') {
      void surfaceController?.setPanel(false).catch(error => showNotice(String(error)));
    }
    if (surface.menuBar && surface.mode === 'docked' && event.type === 'focus') void surfaceController?.setPanel(true).catch(error => showNotice(String(error)));
    presentation = present(presentation, event);
    if (event.type === 'enter' || event.type === 'focus' || event.type === 'surface') clearTimeout(closeTimer);
    if (event.type === 'leave' || event.type === 'blur') {
      clearTimeout(closeTimer);
      closeTimer = setTimeout(() => dispatch({ type: 'close-delay' }), 240);
    }
  }
  onMount(() => {
    let disposed = false;
    const cleanup: (() => void)[] = [];
    const register = (promise: Promise<() => void>) => void promise.then(fn => { if (disposed) fn(); else cleanup.push(fn); }).catch(error => console.error("Pet window listener failed", error));
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const syncTheme = () => { systemDark = media.matches; };
    syncTheme(); media.addEventListener('change', syncTheme);
    let providerRevision = 0;
    register(listen<ProviderConnectionState[]>('provider-connection-status', event => {
      providerRevision++;
      if (!disposed) petData.setProviders(event.payload);
    }).then(async unlisten => {
      const revision = providerRevision;
      try {
        const providers = await invoke<ProviderConnectionState[]>('provider_connection_status');
        if (!disposed && revision === providerRevision) petData.setProviders(providers);
      } catch (error) { console.warn('Pet provider identities unavailable', error); }
      return unlisten;
    }));
    register(listen<AppSettings>('settings-updated', event => { settings = event.payload; }));
    const windowHandle = getCurrentWindow();
    void windowHandle.setResizable(false);
    if (petContentElement) {
      surfaceController = new PetSurfaceController(petContentElement, value => { dispatch({ type: 'surface', surface: value }); if (value.menuBar && value.panelOpen) void setPetWindowCursorEventsIgnored(false); }, error => showNotice(String(error)));
      void surfaceController.start().catch(error => showNotice(String(error)));
      cleanup.push(() => surfaceController?.dispose());
    }
    cursorPassthroughTimer = window.setInterval(() => void syncCursorPassthrough(), cursorPassthroughPollMs);
    void getAppSettings().then(value => { if (!disposed) { settings = value; ready = true; surfaceController?.schedule(); } }).catch(error => { if (!disposed) showNotice(String(error)); });
    void petData.refresh();
    const poll = window.setInterval(() => void petData.refresh(), 1000);
    return () => {
      disposed = true; window.clearInterval(poll); cleanup.forEach(fn => fn()); media.removeEventListener('change', syncTheme);
      menuController.dispose(); petData.dispose(); clearTimeout(closeTimer); clearNoticeTimer(); clearCursorPassthroughTimer(); void setPetWindowCursorEventsIgnored(false);
    };
  });
  function dismissActivity(event: MouseEvent, task: PetTask) { event.stopPropagation(); petData.dismiss(task); }
  function clearCompletedActivities(event: MouseEvent) { event.stopPropagation(); petData.clearCompleted(); }
  function showNotice(message: string) {
    actionNotice = message;
    clearNoticeTimer();
    noticeTimer = window.setTimeout(() => {
      actionNotice = "";
      noticeTimer = null;
    }, noticeVisibleMs);
  }

  function clearNoticeTimer() {
    if (noticeTimer) {
      window.clearTimeout(noticeTimer);
      noticeTimer = null;
    }
  }

  function clearCursorPassthroughTimer() {
    if (cursorPassthroughTimer) {
      window.clearInterval(cursorPassthroughTimer);
      cursorPassthroughTimer = null;
    }
  }

  async function syncCursorPassthrough() {
    if (!petWindowElement || cursorPassthroughInFlight || (surface.menuBar && surface.mode === 'docked' && !surface.panelOpen)) {
      return;
    }

    cursorPassthroughInFlight = true;
    try {
      const point = await invoke<{ x: number; y: number }>('pet_surface_pointer');
      const ignoreCursor = !petDragging && shouldIgnorePetWindowCursor(point, collectPetHitRects(petWindowElement)) && !isPointOnOpaquePetImage(petWindowElement, point);
      await setPetWindowCursorEventsIgnored(ignoreCursor);
    } catch (error) {
      console.error("failed to update pet cursor passthrough", error);
    } finally {
      cursorPassthroughInFlight = false;
    }
  }

  async function setPetWindowCursorEventsIgnored(ignore: boolean) {
    if (cursorEventsIgnored === ignore) {
      return;
    }

    cursorEventsIgnored = ignore;
    try {
      await getCurrentWindow().setIgnoreCursorEvents(ignore);
    } catch (error) {
      cursorEventsIgnored = !ignore;
      console.error("failed to set pet cursor passthrough", error);
    }
  }

  function collectPetHitRects(root: HTMLElement): PetHitRect[] {
    const rootBounds = root.getBoundingClientRect();
    const hitRects: PetHitRect[] = [];
    for (const element of root.querySelectorAll<HTMLElement>(".activity-group-toggle, .status-pill, .pet-action-button, .notch-panel")) {
      if (element.getClientRects().length === 0) continue;
      if (element.closest("[inert]")) continue;

      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        hitRects.push(rectFromElementBounds(rect, rootBounds, petHitPadding));
      }
    }

    return hitRects;
  }

  async function dragWindow() {
    try { await surfaceController?.drag(); }
    catch (error) { showNotice(`无法移动桌宠：${String(error)}`); }
  }

  async function toggleSurface(event: MouseEvent) {
    event.stopPropagation();
    try { await surfaceController?.setMode(surface.mode === 'docked' ? 'floating' : 'docked'); }
    catch (error) { showNotice(String(error)); }
  }

  function handleKey(event: KeyboardEvent) {
    if (event.key === 'Escape' && surface.dragging) void surfaceController?.cancelDrag().catch(error => showNotice(String(error)));
  }

  function preventPetWindowDoubleClick(event: MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
  }

  function toggleTasks(event: MouseEvent) {
    event.stopPropagation();
    dispatch({ type: 'toggle-tasks' });
  }

  async function openMain(event: MouseEvent) {
    event.stopPropagation();
    try {
      await openMainWindow();
      showNotice("已打开主窗口");
    } catch (error) {
      showNotice(String(error));
    }
  }

  function clampPetOpacity(value: number | null | undefined) {
    const numericValue = Number(value);
    if (!Number.isFinite(numericValue)) {
      return defaultPetOpacity;
    }
    return Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue));
  }

</script>

<svelte:window on:keydown={handleKey} />

<main bind:this={petWindowElement}
  class={`pet-window ${themeClass}${devMode ? " dev-mode" : ""}`}
  class:docked={surface.mode === 'docked'} class:snap-candidate={surface.snapCandidate}
  style={`--pet-window-opacity: ${petWindowOpacity}; --pet-rail-height: ${railHeight}px; --pet-desktop-width: ${sizing.desktopWidth}px; --pet-stage-height: ${sizing.stageHeight}px; --notch-header-height: ${sizing.headerHeight}px; --notch-width: ${presentation.expanded ? sizing.expandedWidth : sizing.collapsedWidth}px;`}
  on:dblclick={preventPetWindowDoubleClick}>
  <div class="pet-content" bind:this={petContentElement}>
    {#if surface.mode === 'docked'}
      <NotchPet data={$petData} {settings} {avatarStatus} {surface} {sizing} expanded={presentation.expanded}
        {dispatch} {dragWindow} {toggleSurface} {openMain} {dismissActivity} {actionNotice} />
    {:else}
      <DesktopPet {activities} sources={$petData.sources} gatewayError={$petData.error} {settings} {avatarStatus} {surface} {sizing}
        tasksCollapsed={presentation.tasksCollapsed} bind:expandedGroups {actionNotice}
        {dismissActivity} {clearCompletedActivities} {dragWindow} {toggleSurface} {toggleTasks} {openMain} />
    {/if}
  </div>
</main>

<style>
  .pet-window { justify-content: flex-start; -webkit-user-select: none; }
  .pet-window.docked { width: max-content; max-width: var(--notch-width); opacity: 1; cursor: default; }
  .docked .pet-content { width: max-content; max-width: var(--notch-width); padding: 0 6px; }
  .docked:has(:global(.notch-panel.expanded)) .pet-content { width: var(--notch-width); }
  .snap-candidate :global(.pet-stage) { filter: drop-shadow(0 0 6px var(--color-focus-ring)); }
</style>
