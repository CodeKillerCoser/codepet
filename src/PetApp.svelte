<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { LogicalPosition, LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
  import { availableMonitors, getCurrentWindow, primaryMonitor } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { activateActivity, getAppSettings, openMainWindow, recentEvents, resolveActivityApproval, sendActivityReply } from "./lib/api";
  import { activityCapabilities, activityKey, cardEndTime, cardMessage, cardMeta, cardTitle, primaryActivity, statusLabel, updateActivityList } from "./lib/activity";
  import { runningBubbleStyle } from "./lib/gradientColor";
  import PetAvatar from "./lib/PetAvatar.svelte";
  import { playNotificationSound, playWhipSound, shouldRepeatNotification, shouldRing } from "./lib/sound";
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
  let lastWindowHeight = 0;
  let requestedWindowHeight = 0;
  let syncingWindowFrame = false;
  let replyingToId: string | null = null;
  let replyText = "";
  let actionNotice = "";
  let noticeTimer: number | null = null;
  let whipAnimating = false;
  let whipTimer: number | null = null;

  const petWindowWidth = 360;
  const activityCardHeight = 78;
  const activityGap = 8;
  const activityPetGap = 8;
  const maxVisibleActivities = 4;
  const noticeVisibleMs = 2500;
  const whipVisibleMs = 760;
  const permissionRepeatMaxMs = 590_000;
  const devMode = import.meta.env.DEV;
  const fallbackRunningBubble: AppSettings["appearance"]["runningBubble"] = {
    backgroundBreathing: true,
    borderMarquee: false,
    backgroundColor: "#e8f2ff",
    borderColor: "#3d73d8",
    animationMs: 1800,
  };

  $: themeClass = settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "theme-dark" : "theme-light";
  $: runningBubble = settings?.appearance.runningBubble ?? fallbackRunningBubble;
  $: runningBubbleStyleText = runningBubbleStyle(runningBubble);
  $: primary = primaryActivity(activities);
  $: hasActivities = activities.length > 0;
  $: hasLiveActivities = activities.some((activity) => activity.status === "thinking" || activity.status === "running" || activity.status === "waiting-approval");
  $: showActivities = hasActivities && !tasksCollapsed;
  $: visibleActivities = showActivities ? activities.slice(0, maxVisibleActivities) : [];
  $: activityStackHeight = activityStackHeightFor(visibleActivities, replyingToId);
  $: petScale = Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4);
  $: petVisualHeight = settings?.pet.kind === "codex-atlas" ? Math.round(32 * petScale * (208 / 192)) : 30 * petScale;
  $: petStageHeight = Math.max(104, petVisualHeight);
  $: desiredWindowHeight = petWindowHeight(activityStackHeight, petStageHeight);
  $: topActivityId = showActivities ? activities[0]?.id ?? null : null;
  $: if (!hasActivities && tasksCollapsed) {
    tasksCollapsed = false;
  }
  $: if (hasLiveActivities && tasksCollapsed) {
    tasksCollapsed = false;
  }
  $: if (ready) {
    void syncWindowFrame(desiredWindowHeight);
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
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const syncTheme = () => {
      systemDark = media.matches;
    };
    media.addEventListener("change", syncTheme);

    let disposed = false;
    let unlistenPetEvent: (() => void) | null = null;
    let unlistenSettings: (() => void) | null = null;
    let unlistenAgentDisabled: (() => void) | null = null;

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
      } catch (error) {
        console.error("failed to load pet settings", error);
      }
      if (disposed) {
        return;
      }

      ready = true;
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
      clearRepeat();
      clearPoll();
      clearNoticeTimer();
      clearWhipTimer();
    };
  });

  async function syncLatestFromRecent(ringOnNewEvent: boolean) {
    const nextEvents = await recentEvents();
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

  function applyIncomingEvents(incoming: PetEvent[]) {
    if (incoming.length === 0) {
      return;
    }
    for (const event of incoming) {
      seenEventIds.add(event.id);
    }
    seenEventIds = new Set(seenEventIds);
    activities = updateActivityList(activities, incoming, dismissedActivityKeys, new Date(), hiddenInternalActivityKeys);
    stopRepeatIfNoLongerNeedsAttention();
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

  async function dockToLowerRight() {
    const appWindow = getCurrentWindow();
    const [monitors, fallbackMonitor, size] = await Promise.all([availableMonitors(), primaryMonitor(), appWindow.innerSize()]);
    const visibleMonitors = monitors.filter((monitor) => monitor.workArea.position.y >= 0);
    const monitor = (visibleMonitors.length > 0 ? visibleMonitors : monitors)
      .sort((first, second) => second.workArea.size.width * second.workArea.size.height - first.workArea.size.width * first.workArea.size.height)
      .at(0) ?? fallbackMonitor;
    if (!monitor) {
      return;
    }

    const margin = 42;
    const x = monitor.workArea.position.x + monitor.workArea.size.width - size.width - margin;
    const y = monitor.workArea.position.y + monitor.workArea.size.height - size.height - margin;
    await appWindow.setPosition(new PhysicalPosition(Math.max(x, monitor.workArea.position.x), Math.max(y, monitor.workArea.position.y)));

    const position = await appWindow.outerPosition();
    if (position.y < 0) {
      const fallbackX = Math.max(42, Math.round(window.screen.availLeft + window.screen.availWidth - 402));
      const fallbackY = Math.max(42, Math.round(window.screen.availTop + window.screen.availHeight - 362));
      await appWindow.setPosition(new LogicalPosition(fallbackX, fallbackY));
    }
  }

  async function syncWindowFrame(height: number) {
    requestedWindowHeight = Math.round(height);
    if (syncingWindowFrame) {
      return;
    }

    syncingWindowFrame = true;
    try {
      while (true) {
        const targetHeight = requestedWindowHeight;
        const applied = await applyWindowFrame(targetHeight);
        if (!applied || requestedWindowHeight === targetHeight) {
          break;
        }
      }
    } finally {
      syncingWindowFrame = false;
    }
  }

  async function applyWindowFrame(roundedHeight: number) {
    const appWindow = getCurrentWindow();
    const currentSize = await withTimeout(appWindow.innerSize(), 700).catch(() => null);
    if (requestedWindowHeight !== roundedHeight) {
      return true;
    }
    if (currentSize && roundedHeight === lastWindowHeight && Math.abs(currentSize.height - roundedHeight) <= 1) {
      return true;
    }

    try {
      await withTimeout(appWindow.setSize(new LogicalSize(petWindowWidth, roundedHeight)), 900);
      if (requestedWindowHeight !== roundedHeight) {
        return true;
      }
      lastWindowHeight = roundedHeight;
      void withTimeout(dockToLowerRight(), 1200).catch((error) => {
        console.error("failed to dock pet window", error);
      });
      return true;
    } catch (error) {
      console.error("failed to sync pet window frame", error);
      return false;
    }
  }

  function activityStackHeightFor(currentActivities: PetEvent[], activeReplyingToId: string | null) {
    if (currentActivities.length === 0) {
      return 0;
    }
    return currentActivities.reduce((height, activity, index) => {
      const cardHeight = activity.id === activeReplyingToId ? 110 : activity.status === "waiting-approval" ? 86 : activityCardHeight;
      return height + cardHeight + (index > 0 ? activityGap : 0);
    }, 0);
  }

  function petWindowHeight(stackHeight: number, stageHeight: number) {
    const petBaseHeight = 22 + stageHeight;
    if (stackHeight <= 0) {
      return petBaseHeight;
    }
    return petBaseHeight + activityPetGap + stackHeight;
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
    replyingToId = replyingToId === activity.id ? null : activity.id;
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
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      void sendReply(activity);
    }
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
  on:dblclick={preventPetWindowDoubleClick}
>
  <button class="drag-layer" type="button" aria-label="拖动移动桌宠" on:mousedown={dragWindow}></button>
  {#if showActivities}
    <section class="activity-stack" bind:this={activityStack} aria-live="polite" style={`--pet-activity-stack-height: ${activityStackHeight}px`}>
      {#each visibleActivities as activity (activity.id)}
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
              {#if activity.status === "done" || activity.status === "failed"}
                <button class="dismiss-button inline-dismiss" type="button" aria-label="从列表移除" on:click={(event) => dismissActivity(event, activity)}></button>
              {/if}
            </div>
            <button class="status-open" type="button" aria-label={`打开 ${cardTitle(activity)}`} title={cardMessage(activity)} on:click={() => activate(activity)}>
              <span class="status-message">{cardMessage(activity)}</span>
            </button>
            {#if replyingToId === activity.id}
              <form class="reply-row" on:submit={(event) => submitReply(event, activity)}>
                <input
                  bind:value={replyText}
                  aria-label="回复"
                  placeholder="回复"
                  on:click={(event) => event.stopPropagation()}
                  on:keydown={(event) => handleReplyKeydown(event, activity)}
                />
                <button
                  type="button"
                  on:click={(event) => {
                    event.stopPropagation();
                    void sendReply(activity);
                  }}
                >回复</button>
              </form>
            {/if}
            <div class="status-footer" class:with-actions={capabilities.canApprove || capabilities.canReply}>
              <span class="status-meta" title={cardMeta(activity)}>
                <span class="status-agent">{activity.provider}</span>
                <span class="status-separator"> · </span>
                <span class={`status-state status-${activity.status}`}>{statusLabel(activity.status)}</span>
                {#if endedAt}
                  <span class="status-separator"> · </span>
                  <span class="status-ended-at">{endedAt}</span>
                {/if}
              </span>
              {#if capabilities.canApprove || capabilities.canReply}
                <div class="status-actions" class:approval-mode={capabilities.canApprove} aria-label="任务操作">
                  {#if capabilities.canApprove}
                    <button class="approval-button allow" type="button" aria-label="同意" on:click={(event) => approve(event, activity, "allow")}>
                      <span>同意</span>
                    </button>
                    <button class="approval-button deny" type="button" aria-label="拒绝" on:click={(event) => approve(event, activity, "deny")}>
                      <span>拒绝</span>
                    </button>
                  {/if}
                  {#if capabilities.canReply}
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
    <button
      class="main-window-button"
      type="button"
      aria-label="打开主窗口"
      on:mousedown={(event) => event.stopPropagation()}
      on:click={openMain}
    >
      <span aria-hidden="true"></span>
    </button>
    <button
      class="whip-button"
      type="button"
      aria-label="抽鞭子"
      title="抽鞭子"
      on:mousedown={(event) => event.stopPropagation()}
      on:click={whipPet}
    >
      <span aria-hidden="true"></span>
    </button>
    <svg class="whip-animation whip-svg" class:active={whipAnimating} viewBox="0 0 460 340" aria-hidden="true">
      <defs>
        <linearGradient id="whipHandleGradient" x1="30" y1="296" x2="68" y2="263" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#7a4726" />
          <stop offset="0.45" stop-color="#4f2d1a" />
          <stop offset="1" stop-color="#8d5a34" />
        </linearGradient>
        <linearGradient id="whipFerruleGradient" x1="56" y1="255" x2="75" y2="274" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#f2d08a" />
          <stop offset="0.45" stop-color="#a86f2c" />
          <stop offset="1" stop-color="#5f3a1d" />
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
    <PetAvatar
      sprite={settings?.pet.sprite ?? { body: "#22c55e", accent: "#facc15", eyes: "#0f172a" }}
      kind={settings?.pet.kind}
      imagePath={settings?.pet.imagePath}
      status={primary?.status ?? "idle"}
      scale={Math.min(Math.max(settings?.pet.scale ?? 3, 2), 4)}
    />
    {#if hasActivities}
      <button
        class="fold-button"
        class:collapsed={tasksCollapsed}
        type="button"
        aria-label={tasksCollapsed ? "展开任务列表" : "收起任务列表"}
        on:mousedown={(event) => event.stopPropagation()}
        on:click={toggleTasks}
      >
        <span aria-hidden="true"></span>
      </button>
    {/if}
    {#if actionNotice}
      <span class="pet-notice">{actionNotice}</span>
    {/if}
  </section>
</main>
