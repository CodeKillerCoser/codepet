<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
  import { availableMonitors, getCurrentWindow, primaryMonitor, type Monitor } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import { activateActivity, getAppSettings, openMainWindow, recentEvents, recordPerfEvent, resolveActivityApproval, sendActivityReply } from "./lib/api";
  import { activityCapabilities, activityKey, cardEndTime, cardMessage, cardMeta, cardTitle, matchesActivityFilters, primaryActivity, statusLabel, updateActivityList } from "./lib/activity";
  import { runningBubbleStyle } from "./lib/gradientColor";
  import PetAvatar from "./lib/PetAvatar.svelte";
  import { playNotificationSound, playWhipSound, shouldRepeatNotification, shouldRing } from "./lib/sound";
  import { defaultPetSprite, defaultRunningBubbleSettings, themeClassNames } from "./lib/theme";
  import type { AppSettings, PetEvent } from "./lib/types";

  let settings: AppSettings | null = null;
  let activities: PetEvent[] = [];
  let repeatTimer: number | null = null;
  let repeatEventId: string | null = null;
  let repeatEvent: PetEvent | null = null;
  let repeatExpiresAt = 0;
  let pollTimer: number | null = null;
  let lastEventId: string | null = null;
  let seenEventIds = new Set<string>();
  let dismissedActivityKeys = new Set<string>();
  let hiddenInternalActivityKeys = new Set<string>();
  let systemDark = false;
  let tasksCollapsed = false;
  let activityStack: HTMLElement | null = null;
  let lastTopActivityId: string | null = null;
  let ready = false;
  let replyingToId: string | null = null;
  let replyText = "";
  let actionNotice = "";
  let replyTextarea: HTMLTextAreaElement | null = null;
  let noticeTimer: number | null = null;
  let whipAnimating = false;
  let whipAnimationKey = 0;
  let whipTimer: number | null = null;
  let ensureWindowFrameTimer: number | null = null;
  let ensuringWindowFrame = false;

  const petWindowWidth = 360;
  const activityPetGap = 8;
  const activityStackMaxHeight = 368;
  const maxPetStageHeight = Math.max(104, Math.round(32 * 4 * (208 / 192)));
  const petWindowPresetHeight = 22 + maxPetStageHeight + activityPetGap + activityStackMaxHeight;
  const noticeVisibleMs = 2500;
  const whipVisibleMs = 760;
  const permissionRepeatMaxMs = 590_000;
  const replyEditorMaxRows = 5;
  const devMode = import.meta.env.DEV;
  const fallbackRunningBubble = defaultRunningBubbleSettings;
  const defaultPetOpacity = 1;
  const minPetOpacity = 0.25;

  $: themeClass = themeClassNames(settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "dark" : "light");
  $: runningBubble = settings?.appearance.runningBubble ?? fallbackRunningBubble;
  $: runningBubbleStyleText = runningBubbleStyle(runningBubble);
  $: primary = primaryActivity(activities);
  $: hasActivities = activities.length > 0;
  $: hasLiveActivities = activities.some((activity) => activity.status === "thinking" || activity.status === "running" || activity.status === "waiting-approval");
  $: hasCompletedActivities = activities.some((activity) => activity.status === "done");
  $: showActivities = hasActivities && !tasksCollapsed;
  $: renderedActivities = showActivities ? activities : [];
  $: clearReplyIfNoLongerAvailable(activities, replyingToId);
  $: petScale = Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4);
  $: petWindowOpacity = clampPetOpacity(settings?.pet.opacity);
  $: topActivityId = showActivities ? activities[0]?.id ?? null : null;
  $: if (!hasActivities && tasksCollapsed) {
    tasksCollapsed = false;
  }
  $: if (ready) {
    void ensureWindowFrameAndBounds();
  }
  $: if (activityStack && topActivityId && topActivityId !== lastTopActivityId) {
    lastTopActivityId = topActivityId;
    requestAnimationFrame(() => {
      if (activityStack) {
        activityStack.scrollTop = 0;
      }
    });
  }

  onMount(() => {
    const mountedAt = performance.now();
    const appWindow = getCurrentWindow();
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const syncTheme = () => {
      systemDark = media.matches;
    };
    media.addEventListener("change", syncTheme);
    void appWindow.setResizable(false).catch((error) => {
      console.error("failed to disable pet window resizing", error);
    });

    let disposed = false;
    let unlistenPetEvent: (() => void) | null = null;
    let unlistenSettings: (() => void) | null = null;
    let unlistenAgentDisabled: (() => void) | null = null;
    let unlistenWindowMoved: (() => void) | null = null;
    let unlistenWindowResized: (() => void) | null = null;

    void appWindow.onMoved(() => {
      scheduleEnsureWindowFrameAndBounds();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenWindowMoved = unlisten;
      }
    }).catch((error) => {
      console.error("failed to watch pet window movement", error);
    });

    void appWindow.onResized(() => {
      scheduleEnsureWindowFrameAndBounds();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenWindowResized = unlisten;
      }
    }).catch((error) => {
      console.error("failed to watch pet window resize", error);
    });

    void listen<PetEvent>("pet-event", async (event) => {
      const alreadySeen = seenEventIds.has(event.payload.id) || event.payload.id === lastEventId;
      applyIncomingEvents([event.payload]);
      lastEventId = event.payload.id;
      if (!alreadySeen) {
        await handleRing(event.payload);
      }
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenPetEvent = unlisten;
      }
    }).catch((error) => {
      console.error("failed to listen pet events", error);
    });

    void listen<AppSettings>("settings-updated", (event) => {
      settings = event.payload;
      applyCurrentActivityFilters();
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenSettings = unlisten;
      }
    }).catch((error) => {
      console.error("failed to listen settings updates", error);
    });

    void listen<string>("agent-disabled", (event) => {
      removeActivitiesForAgent(event.payload);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
      } else {
        unlistenAgentDisabled = unlisten;
      }
    }).catch((error) => {
      console.error("failed to listen agent disabled events", error);
    });

    void (async () => {
      try {
        settings = await getAppSettings();
        applyCurrentActivityFilters();
        void recordPerfEvent({
          name: "frontend.pet.get_settings",
          durationMs: performance.now() - mountedAt,
        }).catch(() => {});
      } catch (error) {
        void recordPerfEvent({
          name: "frontend.pet.get_settings",
          status: "error",
          durationMs: performance.now() - mountedAt,
          error: String(error),
        }).catch(() => {});
        console.error("failed to load pet settings", error);
      }
      if (disposed) {
        return;
      }

      ready = true;
      void dockToLowerRight().catch((error) => {
        console.error("failed to dock pet window", error);
      });
      void recordPerfEvent({
        name: "frontend.pet.ready",
        durationMs: performance.now() - mountedAt,
        fields: { activities: activities.length },
      }).catch(() => {});
      void syncLatestFromRecent(false).catch((error) => {
        console.error("failed to load recent pet events", error);
      });
      pollTimer = window.setInterval(() => {
        void syncLatestFromRecent(true).catch((error) => {
          console.error("failed to sync recent pet events", error);
        });
      }, 1000);
    })();

    return () => {
      disposed = true;
      media.removeEventListener("change", syncTheme);
      unlistenPetEvent?.();
      unlistenSettings?.();
      unlistenAgentDisabled?.();
      unlistenWindowMoved?.();
      unlistenWindowResized?.();
      clearRepeat();
      clearPoll();
      clearNoticeTimer();
      clearWhipTimer();
      clearEnsureWindowFrameTimer();
    };
  });

  async function syncLatestFromRecent(ringOnNewEvent: boolean) {
    const startedAt = performance.now();
    const nextEvents = await recentEvents();
    const durationMs = performance.now() - startedAt;
    if (durationMs >= 100 || nextEvents.length > 0) {
      void recordPerfEvent({
        name: "frontend.pet.sync_recent_events",
        durationMs,
        fields: {
          events: nextEvents.length,
          ringOnNewEvent,
        },
      }).catch(() => {});
    }
    const unseenEvents = nextEvents.filter((event) => !seenEventIds.has(event.id));
    applyIncomingEvents(unseenEvents);
    const next = nextEvents.at(-1) ?? null;
    if (!next) {
      lastEventId = null;
      return;
    }

    if (next.id !== lastEventId) {
      lastEventId = next.id;
      if (ringOnNewEvent) {
        await handleRing(next);
      }
    }
  }

  function clearReplyIfNoLongerAvailable(currentActivities: PetEvent[], activeReplyingToId: string | null) {
    if (!activeReplyingToId) {
      return;
    }
    if (currentActivities.some((activity) => activity.id === activeReplyingToId && activityCapabilities(activity).canReply)) {
      return;
    }
    replyingToId = null;
    replyText = "";
  }

  function applyIncomingEvents(incoming: PetEvent[]) {
    if (incoming.length === 0) {
      return;
    }
    const previousLiveKeys = new Set(activities.filter(isLiveActivity).map(activityKey));
    for (const event of incoming) {
      seenEventIds.add(event.id);
    }
    seenEventIds = new Set(seenEventIds);
    const nextActivities = updateActivityList(activities, incoming, dismissedActivityKeys, new Date(), hiddenInternalActivityKeys, settings?.activityFilters);
    const hasNewLiveActivity = nextActivities.some((activity) => isLiveActivity(activity) && !previousLiveKeys.has(activityKey(activity)));
    activities = nextActivities;
    if (tasksCollapsed && hasNewLiveActivity) {
      tasksCollapsed = false;
    }
    stopRepeatIfNoLongerNeedsAttention();
  }

  function applyCurrentActivityFilters() {
    if (!settings?.activityFilters || activities.length === 0) {
      return;
    }
    const nextActivities = activities.filter((activity) => {
      if (!matchesActivityFilters(activity, settings?.activityFilters)) {
        return true;
      }
      hiddenInternalActivityKeys.add(activityKey(activity));
      return false;
    });
    if (nextActivities.length !== activities.length) {
      hiddenInternalActivityKeys = new Set(hiddenInternalActivityKeys);
      activities = nextActivities;
      stopRepeatIfNoLongerNeedsAttention();
    }
  }

  function isLiveActivity(activity: PetEvent) {
    return activity.status === "thinking" || activity.status === "running" || activity.status === "waiting-approval";
  }

  function isActiveActivity(activity: PetEvent) {
    return activity.status === "thinking" || activity.status === "running";
  }

  function removeActivitiesForAgent(agentId: string) {
    activities = activities.filter((activity) => activity.provider !== agentId);
    dismissedActivityKeys = new Set(Array.from(dismissedActivityKeys).filter((key) => !key.startsWith(`${agentId}:`)));
    hiddenInternalActivityKeys = new Set(Array.from(hiddenInternalActivityKeys).filter((key) => !key.startsWith(`${agentId}:`)));
    if (replyingToId && !activities.some((activity) => activity.id === replyingToId)) {
      replyingToId = null;
      replyText = "";
    }
    clearRepeat();
  }

  async function handleRing(event: PetEvent) {
    if (!settings || !shouldRing(settings, event)) {
      clearRepeat();
      return;
    }

    await playNotificationSound(settings);
    clearRepeat();
    if (event.status === "waiting-approval" && settings.notifications.repeatSeconds > 0) {
      repeatEventId = event.id;
      repeatEvent = event;
      repeatExpiresAt = Date.now() + permissionRepeatMaxMs;
      repeatTimer = window.setInterval(() => {
        if (!settings || !repeatEvent || !shouldRepeatNotification(settings, repeatEvent, activities, Date.now(), repeatExpiresAt)) {
          clearRepeat();
          return;
        }
        void playNotificationSound(settings);
      }, settings.notifications.repeatSeconds * 1000);
    }
  }

  function clearRepeat() {
    if (repeatTimer) {
      window.clearInterval(repeatTimer);
      repeatTimer = null;
    }
    repeatEventId = null;
    repeatEvent = null;
    repeatExpiresAt = 0;
  }

  function stopRepeatIfNoLongerNeedsAttention() {
    if (!settings || !repeatEvent) {
      return;
    }
    if (!shouldRepeatNotification(settings, repeatEvent, activities, Date.now(), repeatExpiresAt)) {
      clearRepeat();
    }
  }

  function clearPoll() {
    if (pollTimer) {
      window.clearInterval(pollTimer);
      pollTimer = null;
    }
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

  function dismissActivity(event: MouseEvent, activity: PetEvent) {
    event.stopPropagation();
    dismissedActivityKeys.add(activityKey(activity));
    dismissedActivityKeys = new Set(dismissedActivityKeys);
    activities = activities.filter((candidate) => activityKey(candidate) !== activityKey(activity));
    if (repeatEvent && activityKey(repeatEvent) === activityKey(activity)) {
      clearRepeat();
    }
    if (replyingToId === activity.id) {
      replyingToId = null;
      replyText = "";
    }
  }

  function clearCompletedActivities(event: MouseEvent) {
    event.stopPropagation();
    const completedKeys = new Set(activities.filter((activity) => activity.status === "done").map(activityKey));
    if (completedKeys.size === 0) {
      return;
    }

    for (const key of completedKeys) {
      dismissedActivityKeys.add(key);
    }
    dismissedActivityKeys = new Set(dismissedActivityKeys);
    activities = activities.filter((activity) => !completedKeys.has(activityKey(activity)));
    if (repeatEvent && completedKeys.has(activityKey(repeatEvent))) {
      clearRepeat();
    }
    if (replyingToId && !activities.some((activity) => activity.id === replyingToId)) {
      replyingToId = null;
      replyText = "";
    }
    showNotice("已清除完成任务");
  }

  async function activate(activity: PetEvent) {
    try {
      await activateActivity(activity.id);
      showNotice("已打开来源窗口");
    } catch (error) {
      showNotice(String(error));
    }
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

  function toggleReply(event: MouseEvent, activity: PetEvent) {
    event.stopPropagation();
    if (!activityCapabilities(activity).canReply) {
      showNotice("当前来源不支持可靠回复");
      return;
    }
    const opening = replyingToId !== activity.id;
    replyingToId = opening ? activity.id : null;
    replyText = "";
    if (opening) {
      void focusReplyEditor(activity.id);
    }
  }

  async function focusReplyEditor(activityId: string) {
    await tick();
    if (replyingToId !== activityId || !replyTextarea) {
      return;
    }
    resizeReplyEditor(replyTextarea);
    replyTextarea.focus({ preventScroll: true });
    replyTextarea.setSelectionRange(replyTextarea.value.length, replyTextarea.value.length);
  }

  function cancelReply(event?: Event) {
    event?.preventDefault();
    event?.stopPropagation();
    replyingToId = null;
    replyText = "";
  }

  async function submitReply(event: SubmitEvent, activity: PetEvent) {
    event.preventDefault();
    event.stopPropagation();
    await sendReply(activity);
  }

  async function sendReply(activity: PetEvent) {
    const message = replyText.trim();
    if (!message) {
      return;
    }
    try {
      await sendActivityReply(activity.id, message);
      replyText = "";
      replyingToId = null;
      showNotice("已发送回复");
    } catch (error) {
      showNotice(String(error));
    }
  }

  function handleReplyKeydown(event: KeyboardEvent, activity: PetEvent) {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelReply(event);
      return;
    }
    if (event.key === "Enter" && !event.isComposing && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      void sendReply(activity);
    }
  }

  function handleReplyInput(event: Event) {
    resizeReplyEditor(event.currentTarget as HTMLTextAreaElement);
  }

  function resizeReplyEditor(editor: HTMLTextAreaElement) {
    editor.style.height = "auto";
    const maxHeight = replyEditorMaxHeight(editor);
    const nextHeight = Math.min(editor.scrollHeight, maxHeight);
    editor.style.height = `${nextHeight}px`;
    editor.style.overflowY = editor.scrollHeight > maxHeight ? "auto" : "hidden";
  }

  function replyEditorMaxHeight(editor: HTMLTextAreaElement) {
    const style = window.getComputedStyle(editor);
    const lineHeight = Number.parseFloat(style.lineHeight) || 16;
    const paddingTop = Number.parseFloat(style.paddingTop) || 0;
    const paddingBottom = Number.parseFloat(style.paddingBottom) || 0;
    const borderTop = Number.parseFloat(style.borderTopWidth) || 0;
    const borderBottom = Number.parseFloat(style.borderBottomWidth) || 0;
    return Math.ceil(lineHeight * replyEditorMaxRows + paddingTop + paddingBottom + borderTop + borderBottom);
  }

  function clampPetOpacity(value: number | null | undefined) {
    const numericValue = Number(value);
    if (!Number.isFinite(numericValue)) {
      return defaultPetOpacity;
    }
    return Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue));
  }

  async function approve(event: MouseEvent, activity: PetEvent, behavior: "allow" | "deny") {
    event.stopPropagation();
    try {
      await resolveActivityApproval(activity.id, behavior, behavior === "deny" ? "已在 Code Pet 中拒绝" : undefined);
      clearRepeat();
      showNotice(behavior === "allow" ? "已允许" : "已拒绝");
    } catch (error) {
      showNotice(String(error));
    }
  }
</script>

<main
  class={`pet-window ${themeClass}${devMode ? " dev-mode" : ""}`}
  style={`--pet-window-opacity: ${petWindowOpacity};`}
  on:dblclick={preventPetWindowDoubleClick}
>
  {#if showActivities}
    <section class="activity-stack" bind:this={activityStack} aria-live="polite" style={`--pet-activity-stack-max-height: ${activityStackMaxHeight}px`}>
      {#each renderedActivities as activity (activity.id)}
        {@const capabilities = activityCapabilities(activity)}
        {@const activeActivity = isActiveActivity(activity)}
        {@const endedAt = cardEndTime(activity)}
        <article
          class="status-pill"
          class:active-status={activeActivity}
          class:active-breath={activeActivity && runningBubble.backgroundBreathing}
          class:active-marquee={activeActivity && runningBubble.borderMarquee}
          class:urgent={activity.status === "waiting-approval"}
          class:failed={activity.status === "failed"}
          class:done={activity.status === "done"}
          class:replying={replyingToId === activity.id}
          style={activeActivity ? runningBubbleStyleText : undefined}
          title={`${cardTitle(activity)}\n${cardMessage(activity)}\n${cardMeta(activity)}`}
        >
          <div class="status-content">
            <div class="status-title-row">
              <button class="status-open title-open" type="button" aria-label={`打开 ${cardTitle(activity)}`} title={cardTitle(activity)} on:click={() => activate(activity)}>
                <span>{cardTitle(activity)}</span>
              </button>
              {#if activity.status === "done"}
                <i class="inline-done-mark" aria-hidden="true"></i>
              {/if}
              <button class="dismiss-button inline-dismiss" type="button" aria-label="从列表移除" on:click={(event) => dismissActivity(event, activity)}></button>
            </div>
            <button class="status-open" type="button" aria-label={`打开 ${cardTitle(activity)}`} title={cardMessage(activity)} on:click={() => activate(activity)}>
              <span class="status-message">{cardMessage(activity)}</span>
            </button>
            {#if replyingToId === activity.id}
              <form class="reply-row" on:submit={(event) => submitReply(event, activity)}>
                <textarea
                  bind:this={replyTextarea}
                  bind:value={replyText}
                  aria-label="回复"
                  placeholder="回复"
                  rows="1"
                  on:mousedown={(event) => event.stopPropagation()}
                  on:input={handleReplyInput}
                  on:click={(event) => event.stopPropagation()}
                  on:keydown={(event) => handleReplyKeydown(event, activity)}
                ></textarea>
                <div class="reply-controls">
                  <button
                    class="reply-submit"
                    type="submit"
                    disabled={!replyText.trim()}
                    on:mousedown={(event) => event.stopPropagation()}
                    on:click={(event) => event.stopPropagation()}
                  >发送</button>
                  <button
                    class="reply-cancel"
                    type="button"
                    on:mousedown={(event) => event.stopPropagation()}
                    on:click={cancelReply}
                  >取消</button>
                </div>
              </form>
            {/if}
            <div class="status-footer" class:with-actions={capabilities.canApprove || (capabilities.canReply && replyingToId !== activity.id)}>
              <span class="status-meta" title={cardMeta(activity)}>
                <span class="status-agent">{activity.provider}</span>
                <span class="status-separator"> · </span>
                <span class={`status-state status-${activity.status}`}>{statusLabel(activity.status)}</span>
                {#if endedAt}
                  <span class="status-separator"> · </span>
                  <span class="status-ended-at">{endedAt}</span>
                {/if}
              </span>
              {#if capabilities.canApprove || (capabilities.canReply && replyingToId !== activity.id)}
                <div class="status-actions" class:approval-mode={capabilities.canApprove} aria-label="任务操作">
                  {#if capabilities.canApprove}
                    <button class="approval-button allow" type="button" aria-label="同意" on:click={(event) => approve(event, activity, "allow")}>
                      <span>同意</span>
                    </button>
                    <button class="approval-button deny" type="button" aria-label="拒绝" on:click={(event) => approve(event, activity, "deny")}>
                      <span>拒绝</span>
                    </button>
                  {/if}
                  {#if capabilities.canReply && replyingToId !== activity.id}
                    <button class="reply-button" type="button" on:click={(event) => toggleReply(event, activity)}>回复</button>
                  {/if}
                </div>
              {/if}
            </div>
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
      status={primary?.status ?? "idle"}
      scale={Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4)}
    />
    {#if actionNotice}
      <span class="pet-notice">{actionNotice}</span>
    {/if}
  </section>
</main>
