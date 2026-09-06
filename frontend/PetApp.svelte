<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { LogicalSize, PhysicalPosition } from '@tauri-apps/api/dpi';
  import { availableMonitors, cursorPosition, getCurrentWindow, primaryMonitor, type Monitor } from '@tauri-apps/api/window';
  import { onMount, tick } from 'svelte';
  import { getAppSettings, openMainWindow } from './lib/api';
  import { petSnapshot, petStatusLabel, type PetTask, type PetSource } from './lib/petGateway';
  import { runningBubbleStyle } from './lib/gradientColor';
  import { isOpaqueCssColor, rectFromElementBounds, shouldIgnorePetWindowCursor, type PetHitRect } from './lib/petHitTest';
  import PetAvatar from './lib/PetAvatar.svelte';
  import { playWhipSound } from './lib/sound';
  import { defaultPetSprite, defaultRunningBubbleSettings, themeClassNames } from './lib/theme';
  import type { AppSettings } from './lib/types';
  let settings: AppSettings | null = null;
  let activities: PetTask[] = [];
  let sources: PetSource[] = [];
  let dismissed = new Map<string, number>();
  let gatewayError = '';
  let systemDark = false;
  let tasksCollapsed = false;
  let activityStack: HTMLElement | null = null;
  let lastTopActivityId: string | null = null;
  let ready = false;
  let actionNotice = '';
  let noticeTimer: number | null = null;
  let whipAnimating = false;
  let whipAnimationKey = 0;
  let whipTimer: number | null = null;
  let ensureWindowFrameTimer: number | null = null;
  let ensuringWindowFrame = false;
  let petWindowElement: HTMLElement | null = null;
  let cursorPassthroughTimer: number | null = null;
  let cursorEventsIgnored = false;
  let cursorPassthroughInFlight = false;
  const petImageAlphaCache = new WeakMap<HTMLImageElement, { src: string; width: number; height: number; data: Uint8ClampedArray }>();
  const petWindowWidth = 360;
  const activityPetGap = 8;
  const activityStackMaxHeight = 368;
  const maxPetStageHeight = Math.max(104, Math.round(32 * 4 * (208 / 192)));
  const petWindowPresetHeight = 22 + maxPetStageHeight + activityPetGap + activityStackMaxHeight;
  const noticeVisibleMs = 2500;
  const whipVisibleMs = 760;
  const cursorPassthroughPollMs = 60;
  const petHitPadding = 2;
  const devMode = import.meta.env.DEV;
  const fallbackRunningBubble = defaultRunningBubbleSettings;
  const defaultPetOpacity = 1;
  const minPetOpacity = 0.25;
  $: themeClass = themeClassNames(settings?.appearance.theme === 'dark' || (settings?.appearance.theme === 'system' && systemDark) ? 'dark' : 'light');
  $: runningBubble = settings?.appearance.runningBubble ?? fallbackRunningBubble;
  $: runningBubbleStyleText = runningBubbleStyle(runningBubble);
  $: hasActivities = activities.length > 0;
  $: hasCompletedActivities = activities.some(a => a.status === 'completed');
  $: primary = activities.find(a => a.status === 'running') ?? activities[0];
  $: avatarStatus = primary?.status === 'running' ? 'running' : primary?.status === 'failed' ? 'failed' : primary?.status === 'completed' ? 'done' : primary?.status === 'waiting-approval' ? 'waiting-approval' : 'idle';
  $: petWindowOpacity = clampPetOpacity(settings?.pet.opacity);
  $: topActivityId = activities[0]?.id ?? null;
  $: if (ready) scheduleEnsureWindowFrameAndBounds();
  $: if (activityStack && topActivityId !== lastTopActivityId) { lastTopActivityId = topActivityId; void tick().then(() => { if (activityStack) activityStack.scrollTop = 0; }); }
  onMount(() => {
    let disposed = false;
    let busy = false;
    const cleanup: (() => void)[] = [];
    const register = (promise: Promise<() => void>) => void promise.then(fn => { if (disposed) fn(); else cleanup.push(fn); }).catch(error => console.error("Pet window listener failed", error));
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const syncTheme = () => { systemDark = media.matches; };
    syncTheme(); media.addEventListener('change', syncTheme);
    register(listen<AppSettings>('settings-updated', event => { settings = event.payload; }));
    const windowHandle = getCurrentWindow();
    void windowHandle.setResizable(false);
    register(windowHandle.onMoved(() => scheduleEnsureWindowFrameAndBounds()));
    register(windowHandle.onResized(() => scheduleEnsureWindowFrameAndBounds()));
    cursorPassthroughTimer = window.setInterval(() => void syncCursorPassthrough(), cursorPassthroughPollMs);
    const refresh = async () => {
      if (busy || disposed) return;
      busy = true;
      try {
        const snapshot = await petSnapshot();
        if (disposed) return;
        sources = snapshot.sources ?? [];
        const previousLive = new Set(activities.filter(a => ["running", "waiting-approval", "waiting-input"].includes(a.status)).map(a => a.id));
        const hasNewLive = snapshot.tasks.some(a => ["running", "waiting-approval", "waiting-input"].includes(a.status) && !previousLive.has(a.id) && (dismissed.get(a.id) ?? -1) < a.updatedAt);
        if (hasNewLive) tasksCollapsed = false;
        activities = snapshot.tasks.filter(a => (dismissed.get(a.id) ?? -1) < a.updatedAt);
        gatewayError = '';
      } catch (error) { if (!disposed) { gatewayError = String(error); activities = activities.map(a => ["running", "waiting-approval", "waiting-input"].includes(a.status) ? { ...a, status: "unknown" } : a); } }
      finally { busy = false; }
    };
    void getAppSettings().then(value => { if (!disposed) { settings = value; ready = true; void dockToLowerRight(); } }).catch(error => { if (!disposed) showNotice(String(error)); });
    void refresh();
    const poll = window.setInterval(() => void refresh(), 1000);
    return () => {
      disposed = true; window.clearInterval(poll); cleanup.forEach(fn => fn()); media.removeEventListener('change', syncTheme);
      clearNoticeTimer(); clearWhipTimer(); clearEnsureWindowFrameTimer(); clearCursorPassthroughTimer(); void setPetWindowCursorEventsIgnored(false);
    };
  });
  function dismissActivity(event: MouseEvent, task: PetTask) {
    event.stopPropagation(); dismissed.set(task.id, task.updatedAt); activities = activities.filter(a => a.id !== task.id);
  }
  function clearCompletedActivities(event: MouseEvent) {
    event.stopPropagation(); activities.filter(a => a.status === 'completed').forEach(a => dismissed.set(a.id, a.updatedAt));
    activities = activities.filter(a => a.status !== 'completed');
  }
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

  function clearWhipTimer() {
    if (whipTimer) {
      window.clearTimeout(whipTimer);
      whipTimer = null;
    }
  }

  function clearEnsureWindowFrameTimer() {
    if (ensureWindowFrameTimer) {
      window.clearTimeout(ensureWindowFrameTimer);
      ensureWindowFrameTimer = null;
    }
  }

  function clearCursorPassthroughTimer() {
    if (cursorPassthroughTimer) {
      window.clearInterval(cursorPassthroughTimer);
      cursorPassthroughTimer = null;
    }
  }

  async function syncCursorPassthrough() {
    if (!petWindowElement || cursorPassthroughInFlight) {
      return;
    }

    cursorPassthroughInFlight = true;
    try {
      const appWindow = getCurrentWindow();
      const [cursor, windowPosition, scaleFactor] = await Promise.all([
        cursorPosition(),
        appWindow.outerPosition(),
        appWindow.scaleFactor(),
      ]);
      const point = {
        x: (cursor.x - windowPosition.x) / scaleFactor,
        y: (cursor.y - windowPosition.y) / scaleFactor,
      };
      const ignoreCursor = shouldIgnorePetWindowCursor(point, collectPetHitRects(petWindowElement)) && !isPointOnOpaquePetImage(petWindowElement, point);
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
    for (const element of root.querySelectorAll<HTMLElement>(".status-pill, .pet-action-button")) {
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        hitRects.push(rectFromElementBounds(rect, rootBounds, petHitPadding));
      }
    }

    for (const pixel of root.querySelectorAll<HTMLElement>(".pet-sprite span")) {
      if (!isOpaqueCssColor(window.getComputedStyle(pixel).backgroundColor || pixel.style.backgroundColor || pixel.style.background)) {
        continue;
      }
      const rect = pixel.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        hitRects.push(rectFromElementBounds(rect, rootBounds));
      }
    }

    return hitRects;
  }

  function isPointOnOpaquePetImage(root: HTMLElement, point: { x: number; y: number }) {
    const rootBounds = root.getBoundingClientRect();
    for (const wrapper of root.querySelectorAll<HTMLElement>(".pet-image, .pet-atlas")) {
      const wrapperRect = rectFromElementBounds(wrapper.getBoundingClientRect(), rootBounds, petHitPadding);
      if (shouldIgnorePetWindowCursor(point, [wrapperRect])) {
        continue;
      }

      const image = wrapper instanceof HTMLImageElement ? wrapper : wrapper.querySelector("img");
      if (!image || !image.complete || image.naturalWidth <= 0 || image.naturalHeight <= 0) {
        return true;
      }

      const alpha = petImageAlphaAtPoint(wrapper, image, point, rootBounds);
      if (alpha === null) {
        return true;
      }
      if (alpha > 12) {
        return true;
      }
    }

    return false;
  }

  function petImageAlphaAtPoint(wrapper: HTMLElement, image: HTMLImageElement, point: { x: number; y: number }, rootBounds: DOMRect) {
    const pixel = petImagePixelCoordinates(wrapper, image, point, rootBounds);
    if (!pixel) {
      return null;
    }

    const data = petImageAlphaData(image);
    if (!data) {
      return null;
    }

    const x = Math.floor(pixel.x);
    const y = Math.floor(pixel.y);
    if (x < 0 || y < 0 || x >= data.width || y >= data.height) {
      return 0;
    }

    return data.data[(y * data.width + x) * 4 + 3];
  }

  function petImagePixelCoordinates(wrapper: HTMLElement, image: HTMLImageElement, point: { x: number; y: number }, rootBounds: DOMRect) {
    const wrapperRect = rectFromElementBounds(wrapper.getBoundingClientRect(), rootBounds);
    const relativeX = point.x - wrapperRect.left;
    const relativeY = point.y - wrapperRect.top;
    const wrapperWidth = wrapperRect.right - wrapperRect.left;
    const wrapperHeight = wrapperRect.bottom - wrapperRect.top;
    if (wrapperWidth <= 0 || wrapperHeight <= 0) {
      return null;
    }
    if (wrapper instanceof HTMLImageElement) {
      return {
        x: relativeX * (image.naturalWidth / wrapperWidth),
        y: relativeY * (image.naturalHeight / wrapperHeight),
      };
    }

    const transform = getComputedStyle(image).transform;
    const matrix = transform && transform !== "none" ? new DOMMatrixReadOnly(transform) : new DOMMatrixReadOnly();
    const imageWidth = image.offsetWidth || image.getBoundingClientRect().width;
    const imageHeight = image.offsetHeight || image.getBoundingClientRect().height;
    if (imageWidth <= 0 || imageHeight <= 0) {
      return null;
    }

    return {
      x: (relativeX - matrix.m41) * (image.naturalWidth / imageWidth),
      y: (relativeY - matrix.m42) * (image.naturalHeight / imageHeight),
    };
  }

  function petImageAlphaData(image: HTMLImageElement) {
    const cached = petImageAlphaCache.get(image);
    if (cached && cached.src === image.currentSrc && cached.width === image.naturalWidth && cached.height === image.naturalHeight) {
      return cached;
    }

    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) {
      return null;
    }

    try {
      context.drawImage(image, 0, 0);
      const next = {
        src: image.currentSrc,
        width: image.naturalWidth,
        height: image.naturalHeight,
        data: context.getImageData(0, 0, image.naturalWidth, image.naturalHeight).data,
      };
      petImageAlphaCache.set(image, next);
      return next;
    } catch {
      return null;
    }
  }

  function scheduleEnsureWindowFrameAndBounds() {
    clearEnsureWindowFrameTimer();
    ensureWindowFrameTimer = window.setTimeout(() => {
      ensureWindowFrameTimer = null;
      void ensureWindowFrameAndBounds().catch((error) => {
        console.error("failed to ensure pet window frame", error);
      });
    }, 120);
  }

  async function dockToLowerRight() {
    const appWindow = getCurrentWindow();
    await ensureWindowSize();
    const [monitors, fallbackMonitor, position, size] = await Promise.all([availableMonitors(), primaryMonitor(), appWindow.outerPosition(), appWindow.outerSize()]);
    const monitor = monitorForWindow(position, size, monitors, fallbackMonitor);
    if (!monitor) {
      return;
    }

    const margin = 42;
    const x = monitor.workArea.position.x + monitor.workArea.size.width - size.width - margin;
    const y = monitor.workArea.position.y + monitor.workArea.size.height - size.height - margin;
    const clamped = clampWindowPositionToMonitor({ x, y }, size, monitor);
    await appWindow.setPosition(new PhysicalPosition(clamped.x, clamped.y));

    await ensureWindowFrameAndBounds();
  }

  async function ensureWindowFrameAndBounds() {
    if (ensuringWindowFrame) {
      return;
    }

    ensuringWindowFrame = true;
    try {
      await ensureWindowSize();
      await constrainWindowToScreen();
    } finally {
      ensuringWindowFrame = false;
    }
  }

  async function ensureWindowSize() {
    const targetHeight = petWindowPresetHeight;
    if (targetHeight <= 0) {
      return;
    }

    const appWindow = getCurrentWindow();
    const [currentSize, scaleFactor] = await Promise.all([
      withTimeout(appWindow.innerSize(), 700).catch(() => null),
      withTimeout(appWindow.scaleFactor(), 700).catch(() => 1),
    ]);
    if (!currentSize) {
      return;
    }

    const currentLogicalSize = currentSize.toLogical(scaleFactor);
    if (Math.abs(currentLogicalSize.width - petWindowWidth) > 1 || Math.abs(currentLogicalSize.height - targetHeight) > 1) {
      await appWindow.setSize(new LogicalSize(petWindowWidth, targetHeight));
    }
  }

  async function constrainWindowToScreen() {
    const appWindow = getCurrentWindow();
    const [monitors, fallbackMonitor, position, size] = await Promise.all([availableMonitors(), primaryMonitor(), appWindow.outerPosition(), appWindow.outerSize()]);
    const monitor = monitorForWindow(position, size, monitors, fallbackMonitor);
    if (!monitor) {
      return;
    }

    const clamped = clampWindowPositionToMonitor(position, size, monitor);
    if (Math.round(clamped.x) !== Math.round(position.x) || Math.round(clamped.y) !== Math.round(position.y)) {
      await appWindow.setPosition(new PhysicalPosition(clamped.x, clamped.y));
    }
  }

  function monitorForWindow(
    position: { x: number; y: number },
    size: { width: number; height: number },
    monitors: Monitor[],
    fallbackMonitor: Monitor | null,
  ) {
    const windowRight = position.x + size.width;
    const windowBottom = position.y + size.height;
    let bestMonitor: Monitor | null = null;
    let bestIntersection = 0;

    for (const monitor of monitors) {
      const area = monitor.workArea;
      const intersectionWidth = Math.max(0, Math.min(windowRight, area.position.x + area.size.width) - Math.max(position.x, area.position.x));
      const intersectionHeight = Math.max(0, Math.min(windowBottom, area.position.y + area.size.height) - Math.max(position.y, area.position.y));
      const intersection = intersectionWidth * intersectionHeight;
      if (intersection > bestIntersection) {
        bestIntersection = intersection;
        bestMonitor = monitor;
      }
    }

    if (bestMonitor) {
      return bestMonitor;
    }

    const center = { x: position.x + size.width / 2, y: position.y + size.height / 2 };
    return monitors
      .slice()
      .sort((first, second) => distanceToMonitorCenter(center, first) - distanceToMonitorCenter(center, second))
      .at(0) ?? fallbackMonitor;
  }

  function distanceToMonitorCenter(point: { x: number; y: number }, monitor: Monitor) {
    const area = monitor.workArea;
    const centerX = area.position.x + area.size.width / 2;
    const centerY = area.position.y + area.size.height / 2;
    return (point.x - centerX) ** 2 + (point.y - centerY) ** 2;
  }

  function clampWindowPositionToMonitor(
    position: { x: number; y: number },
    size: { width: number; height: number },
    monitor: Monitor,
  ) {
    const area = monitor.workArea;
    const minX = area.position.x;
    const minY = area.position.y;
    const maxX = area.position.x + Math.max(0, area.size.width - size.width);
    const maxY = area.position.y + Math.max(0, area.size.height - size.height);
    return {
      x: Math.min(Math.max(position.x, minX), maxX),
      y: Math.min(Math.max(position.y, minY), maxY),
    };
  }

  function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
    return new Promise((resolve, reject) => {
      const timeout = window.setTimeout(() => reject(new Error(`operation timed out after ${timeoutMs}ms`)), timeoutMs);
      promise.then(
        (value) => {
          window.clearTimeout(timeout);
          resolve(value);
        },
        (error) => {
          window.clearTimeout(timeout);
          reject(error);
        },
      );
    });
  }

  async function dragWindow(event: MouseEvent) {
    if (event.button !== 0) {
      return;
    }
    await getCurrentWindow().startDragging();
  }

  function preventPetWindowDoubleClick(event: MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
  }

  function toggleTasks(event: MouseEvent) {
    event.stopPropagation();
    tasksCollapsed = !tasksCollapsed;
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

  function clampPetOpacity(value: number | null | undefined) {
    const numericValue = Number(value);
    if (!Number.isFinite(numericValue)) {
      return defaultPetOpacity;
    }
    return Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue));
  }

</script>

<main
  bind:this={petWindowElement}
  class={`pet-window ${themeClass}${devMode ? " dev-mode" : ""}`}
  style={`--pet-window-opacity: ${petWindowOpacity};`}
  on:dblclick={preventPetWindowDoubleClick}
>
  {#if !tasksCollapsed}
    <section class="activity-stack" bind:this={activityStack} aria-live="polite" style={`--pet-activity-stack-max-height: ${activityStackMaxHeight}px`}>
      {#if activities.length === 0}
        <article class="status-pill gateway-state-pill"><div class="status-content">
          <span class="status-title">{gatewayError ? "活动连接不可用" : "暂无任务"}</span>
          <span class="status-message">{gatewayError || sources.filter(s => s.enabled).map(s => `${s.displayName}：${s.message || "等待活动"}`).join(" · ") || "请在主窗口启用活动来源"}</span>
        </div></article>
      {/if}
      {#each activities as activity (activity.id)}
        <article class="status-pill" class:active-status={activity.status === "running"}
          class:active-breath={activity.status === "running" && runningBubble.backgroundBreathing}
          class:active-marquee={activity.status === "running" && runningBubble.borderMarquee}
          class:urgent={activity.status === "waiting-approval" || activity.status === "waiting-input"}
          class:failed={activity.status === "failed"} class:done={activity.status === "completed"}
          style={activity.status === "running" ? runningBubbleStyleText : undefined}>
          <div class="status-content">
            <div class="status-title-row"><span class="status-title"><span title={activity.title}>{activity.title}</span></span>
              <button class="dismiss-button" type="button" aria-label={`隐藏 ${activity.title}`} on:click={(event) => dismissActivity(event, activity)}></button>
            </div>
            <span class="status-message" title={activity.summary || activity.cwd || ""}>{activity.summary || activity.toolName || activity.cwd || ""}</span>
            <div class="status-footer"><span class="status-meta">{sources.find(s => s.id === activity.providerId)?.displayName || activity.providerId} · {petStatusLabel(activity.status)} · <time class="status-ended-at" datetime={new Date(activity.updatedAt).toISOString()}>{new Date(activity.updatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time></span></div>
          </div>
        </article>
      {/each}
    </section>
  {/if}
  <section class="pet-stage" aria-label="拖动移动桌宠">
    <button class="pet-drag-target" data-pet-hit-target="stage" type="button" tabindex="-1" aria-label="拖动移动桌宠" on:mousedown={dragWindow}></button>
    <div class="pet-action-rail">
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
      sprite={settings?.pet.sprite ?? defaultPetSprite}
      kind={settings?.pet.kind}
      imagePath={settings?.pet.imagePath}
      status={avatarStatus}
      scale={Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4)}
    />
    {#if actionNotice}
      <span class="pet-notice">{actionNotice}</span>
    {/if}
  </section>
</main>

<style>
  .status-pill { cursor: default; }
  .status-title-row { grid-template-columns: minmax(0, 1fr) auto; }
  .status-title { min-width: 0; display: grid; }
  .dismiss-button::before, .dismiss-button::after { left: 5px; top: 8px; }
</style>
