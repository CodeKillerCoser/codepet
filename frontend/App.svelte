<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    Activity,
    BarChart3,
    Bell,
    Bot,
    Check,
    Clock3,
    Filter,
    FolderCog,
    FolderOpen,
    ImagePlus,
    Moon,
    Palette,
    PlugZap,
    Power,
    RotateCcw,
    Rocket,
    ShieldAlert,
    Sun,
    Trash2,
    Volume2,
  } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { appDataDirectory, collectorEndpoint, cutOutImageSubject, deletePet, getAppSettings, getLaunchAtLoginEnabled, importPetImage, listAgents, listPets, recentEvents, recordPerfEvent, selectPet, setAgentEnabled, setAgentHookEvents, setAppDataDirectory, setLaunchAtLoginEnabled, setPetDataDirectory, tokenUsageSummary, updateAppSettings, updatePetImagePixelSize } from "./lib/api";
  import { colorStopIndexFromBand, updateRunningBubbleColorSetting, type RunningBubbleColorKey } from "./lib/bubbleColorSettings";
  import { mergeEventFeed } from "./lib/eventFeed";
  import { gradientEditorFromCss, gradientSegmentCss, nextGradientStopColor, type GradientEditorValue } from "./lib/gradientColor";
  import PetAvatar from "./lib/PetAvatar.svelte";
  import { playNotificationSound, playWhipReactionSound } from "./lib/sound";
  import { defaultRunningBubbleSettings, themeClassNames } from "./lib/theme";
  import { buildUsageChartData, yAxisTicks, type UsageBucketSize, type UsageRange } from "./lib/usageChart";
  import type { ActivityKeywordFilterSettings, AgentId, AgentView, AppSettings, PetEvent, PetLibraryView, TokenUsageSummary } from "./lib/types";

  type ActivityFilterKind = keyof ActivityKeywordFilterSettings;

  let tab: "agents" | "usage" | "personalize" | "events" = "agents";
  let agents: AgentView[] = [];
  let settings: AppSettings | null = null;
  let petLibrary: PetLibraryView | null = null;
  let usage: TokenUsageSummary | null = null;
  let events: PetEvent[] = [];
  let endpoint = "";
  let appDataDir = "";
  let busyAgent: string | null = null;
  let busyPet = "";
  let busyAppDataDirectory = false;
  let appDataRestartPending = false;
  let busyLaunchAtLogin = false;
  let error = "";
  let launchAtLogin = false;
  let systemDark = false;
  let eventPollTimer: number | null = null;
  let runningBubbleSaveToken = 0;
  let runningBubbleSaveTimer: number | null = null;
  let selectedBubbleColorStop: Record<RunningBubbleColorKey, number> = {
    backgroundColor: 0,
    borderColor: 0,
  };
  let usageRange: UsageRange = "7d";
  let usageBucketSize: UsageBucketSize = "30m";
  const agentOrder: AgentId[] = ["codex", "claude", "qoder", "cursor"];
  let filterDrafts: Record<AgentId, Record<ActivityFilterKind, string>> = createFilterDrafts();
  const usageRanges: Array<{ value: UsageRange; label: string }> = [
    { value: "24h", label: "24小时" },
    { value: "7d", label: "7天" },
    { value: "30d", label: "30天" },
    { value: "90d", label: "90天" },
    { value: "1y", label: "近一年" },
  ];
  const usageBucketSizes: Array<{ value: UsageBucketSize; label: string }> = [
    { value: "30m", label: "30分钟" },
    { value: "1h", label: "1小时" },
    { value: "5h", label: "5小时" },
    { value: "12h", label: "12小时" },
    { value: "24h", label: "24小时" },
  ];
  const whipReactionSounds: Array<{ value: AppSettings["pet"]["whipReactionSound"]; label: string }> = [
    { value: "none", label: "无" },
    { value: "pa", label: "啪" },
    { value: "scream", label: "啊啊啊" },
    { value: "custom", label: "自定义" },
  ];
  const runningBubbleDefaults = defaultRunningBubbleSettings;
  const agentActivityFilterDefaults: ActivityKeywordFilterSettings = {
    titleKeywords: [],
    messageKeywords: [],
  };
  const activityFilterDefaults: AppSettings["activityFilters"] = {
    titleKeywords: [],
    messageKeywords: [],
    byAgent: {},
  };
  const agentSettingsDefaults: AppSettings["agents"] = {
    byAgent: {},
  };
  const activityFilterGroups = [
    { key: "titleKeywords", label: "标题", placeholder: "添加标题关键字" },
    { key: "messageKeywords", label: "内容", placeholder: "添加内容关键字" },
  ] as const;
  const bubbleColorConfigs = [
    { key: "backgroundColor", label: "背景色", fallback: runningBubbleDefaults.backgroundColor, directional: true },
    { key: "borderColor", label: "边框色", fallback: runningBubbleDefaults.borderColor, directional: false },
  ] as const;
  const defaultImagePixelSize = 48;
  const defaultPetOpacity = 1;
  const minPetOpacity = 0.25;

  onMount(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const syncTheme = () => {
      systemDark = media.matches;
    };
    media.addEventListener("change", syncTheme);

    let disposed = false;
    let unlistenPetEvent: (() => void) | null = null;
    let unlistenTokenUsage: (() => void) | null = null;
    let unlistenAgentDisabled: (() => void) | null = null;
    let unlistenSettings: (() => void) | null = null;
    void (async () => {
      await keepWindowVisible();
      unlistenPetEvent = await listen<PetEvent>("pet-event", (event) => {
        events = mergeEventFeed(events, [event.payload]);
      });
      unlistenTokenUsage = await listen<TokenUsageSummary>("token-usage-updated", (event) => {
        usage = event.payload;
      });
      unlistenAgentDisabled = await listen<string>("agent-disabled", (event) => {
        events = events.filter((activity) => activity.provider !== event.payload);
      });
      unlistenSettings = await listen<AppSettings>("settings-updated", (event) => {
        settings = normalizeSettings(event.payload);
      });
      if (disposed) {
        unlistenPetEvent();
        unlistenTokenUsage();
        unlistenAgentDisabled();
        unlistenSettings();
        return;
      }

      await refresh();
      eventPollTimer = window.setInterval(() => {
        void syncRecentEvents();
      }, 8000);
    })();

    return () => {
      disposed = true;
      media.removeEventListener("change", syncTheme);
      unlistenPetEvent?.();
      unlistenTokenUsage?.();
      unlistenAgentDisabled?.();
      unlistenSettings?.();
      clearEventPoll();
      clearRunningBubbleSaveTimer();
    };
  });

  async function keepWindowVisible() {
    const appWindow = getCurrentWindow();
    const position = await appWindow.outerPosition();
    if (position.y < 0) {
      const fallbackX = Math.max(42, Math.round(window.screen.availLeft + 80));
      const fallbackY = Math.max(42, Math.round(window.screen.availTop + 80));
      await appWindow.setPosition(new LogicalPosition(fallbackX, fallbackY));
    }
  }

  async function refresh() {
    error = "";
    const startedAt = performance.now();
    try {
      const [nextAgents, nextEvents, nextEndpoint, nextAppDataDir, nextPetLibrary, nextUsage, nextLaunchAtLogin] = await Promise.all([
        measureFrontendPerf("frontend.main.list_agents", () => listAgents()),
        measureFrontendPerf("frontend.main.recent_events", () => recentEvents()),
        measureFrontendPerf("frontend.main.collector_endpoint", () => collectorEndpoint()),
        measureFrontendPerf("frontend.main.app_data_directory", () => appDataDirectory()),
        measureFrontendPerf("frontend.main.list_pets", () => listPets()),
        measureFrontendPerf("frontend.main.token_usage_summary", () => tokenUsageSummary()),
        measureFrontendPerf("frontend.main.get_launch_at_login", () => getLaunchAtLoginEnabled()),
      ]);
      agents = nextAgents;
      events = mergeEventFeed(events, nextEvents);
      endpoint = nextEndpoint;
      appDataDir = nextAppDataDir;
      petLibrary = nextPetLibrary;
      usage = nextUsage;
      launchAtLogin = nextLaunchAtLogin;
      settings = normalizeSettings(await measureFrontendPerf("frontend.main.get_settings", () => getAppSettings()));
      void recordPerfEvent({
        name: "frontend.main.refresh",
        durationMs: performance.now() - startedAt,
        fields: {
          agents: nextAgents.length,
          events: nextEvents.length,
          pets: nextPetLibrary.pets.length,
        },
      }).catch(() => {});
    } catch (currentError) {
      void recordPerfEvent({
        name: "frontend.main.refresh",
        status: "error",
        durationMs: performance.now() - startedAt,
        error: String(currentError),
      }).catch(() => {});
      error = String(currentError);
    }
  }

  async function measureFrontendPerf<T>(name: string, task: () => Promise<T>): Promise<T> {
    const startedAt = performance.now();
    try {
      const value = await task();
      void recordPerfEvent({ name, durationMs: performance.now() - startedAt }).catch(() => {});
      return value;
    } catch (currentError) {
      void recordPerfEvent({
        name,
        status: "error",
        durationMs: performance.now() - startedAt,
        error: String(currentError),
      }).catch(() => {});
      throw currentError;
    }
  }

  async function syncRecentEvents() {
    try {
      events = mergeEventFeed(events, await recentEvents());
    } catch (currentError) {
      error = String(currentError);
    }
  }

  function clearEventPoll() {
    if (eventPollTimer) {
      window.clearInterval(eventPollTimer);
      eventPollTimer = null;
    }
  }

  async function toggleAgent(agent: AgentView) {
    busyAgent = agent.id;
    error = "";
    try {
      agents = await setAgentEnabled(agent.id, !agent.enabled);
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyAgent = null;
    }
  }

  function agentBusy(agent: AgentView) {
    return busyAgent === agent.id || busyAgent === hookBusyKey(agent.id);
  }

  function hookBusyKey(agentId: AgentId) {
    return `${agentId}:hooks`;
  }

  function selectedHookEvents(agent: AgentView) {
    return normalizeHookEventsForAgent(agent, agent.selectedHookEvents);
  }

  function hookEventSelected(agent: AgentView, hookEvent: string) {
    return selectedHookEvents(agent).includes(hookEvent);
  }

  function isLastSelectedHookEvent(agent: AgentView, hookEvent: string) {
    const selectedEvents = selectedHookEvents(agent);
    return selectedEvents.length <= 1 && selectedEvents.includes(hookEvent);
  }

  async function toggleHookEvent(agent: AgentView, hookEvent: string, checked: boolean) {
    const currentEvents = selectedHookEvents(agent);
    const nextEvents = checked
      ? agent.hookEvents.filter((event) => event === hookEvent || currentEvents.includes(event))
      : currentEvents.filter((event) => event !== hookEvent);
    if (nextEvents.length === 0) {
      return;
    }

    busyAgent = hookBusyKey(agent.id);
    error = "";
    try {
      agents = await setAgentHookEvents(agent.id, nextEvents);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyAgent = null;
    }
  }

  async function saveSettings() {
    if (!settings) return;
    normalizeSettings(settings);
    syncSelectedPetProfile();
    settings = await updateAppSettings(settings);
    petLibrary = {
      dataDirectory: petLibrary?.dataDirectory ?? settings.petLibrary.dataDirectory ?? "",
      selectedPetId: settings.petLibrary.selectedPetId,
      pets: settings.petLibrary.pets,
    };
  }

  async function setTheme(theme: AppSettings["appearance"]["theme"]) {
    if (!settings) return;
    settings.appearance.theme = theme;
    await saveSettings();
  }

  async function toggleLaunchAtLogin() {
    busyLaunchAtLogin = true;
    error = "";
    try {
      launchAtLogin = await setLaunchAtLoginEnabled(!launchAtLogin);
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyLaunchAtLogin = false;
    }
  }

  async function pickCustomSound() {
    if (!settings) return;
    const selected = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["mp3", "wav", "m4a", "aac", "ogg"] }],
    });
    if (typeof selected === "string") {
      settings.notifications.customSoundPath = selected;
      settings.notifications.sound = "custom";
      await saveSettings();
    }
  }

  async function pickCustomWhipReactionSound() {
    if (!settings) return;
    const selected = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["mp3", "wav", "m4a", "aac", "ogg"] }],
    });
    if (typeof selected === "string") {
      settings.pet.customWhipReactionSoundPath = selected;
      settings.pet.whipReactionSound = "custom";
      await saveSettings();
    }
  }

  async function importImagePet(cutOutSubject = false) {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp"] }],
    });
    if (typeof selected !== "string") return;

    busyPet = cutOutSubject ? "cutout-import" : "import";
    error = "";
    try {
      const filename = selected.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "Imported Pet";
      const sourcePath = cutOutSubject ? (await cutOutImageSubject(selected, cutoutOutputPath(selected))).outputPath : selected;
      petLibrary = await importPetImage(sourcePath, filename, settings?.pet.imagePixelSize ?? defaultImagePixelSize);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  function cutoutOutputPath(sourcePath: string) {
    const baseDirectory = petLibrary?.dataDirectory ?? settings?.petLibrary.dataDirectory ?? "";
    const filename = sourcePath.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "subject";
    const separator = baseDirectory.includes("\\") ? "\\" : "/";
    const timestamp = Date.now();
    return baseDirectory
      ? `${baseDirectory}${separator}cutouts${separator}${filename}-${timestamp}.png`
      : undefined;
  }

  async function choosePetDataDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;

    busyPet = "directory";
    error = "";
    try {
      petLibrary = await setPetDataDirectory(selected);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function chooseAppDataDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    await updateAppDataDirectory(selected);
  }

  async function resetAppDataDirectory() {
    await updateAppDataDirectory(null);
  }

  async function updateAppDataDirectory(path: string | null) {
    busyAppDataDirectory = true;
    error = "";
    try {
      settings = normalizeSettings(await setAppDataDirectory(path));
      appDataDir = await appDataDirectory();
      petLibrary = await listPets();
      usage = await tokenUsageSummary();
      appDataRestartPending = true;
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyAppDataDirectory = false;
    }
  }

  async function activatePet(petId: string) {
    busyPet = petId;
    error = "";
    try {
      petLibrary = await selectPet(petId);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function removePet(event: MouseEvent, petId: string) {
    event.stopPropagation();
    if (petId === "default" || !window.confirm("删除这个宠物？")) return;

    busyPet = `delete:${petId}`;
    error = "";
    try {
      petLibrary = await deletePet(petId);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  function syncSelectedPetProfile() {
    if (!settings) return;
    settings.petLibrary.selectedPetId = settings.pet.selectedPetId;
    const selected = settings.petLibrary.pets.find((pet) => pet.id === settings?.pet.selectedPetId);
    if (!selected) return;
    selected.sprite = settings.pet.sprite;
    selected.imagePath = settings.pet.imagePath;
  }

  function normalizeSettings(nextSettings: AppSettings) {
    nextSettings.data = {
      ...(nextSettings.data ?? {}),
    };
    nextSettings.appearance.runningBubble = {
      ...runningBubbleDefaults,
      ...(nextSettings.appearance.runningBubble ?? {}),
    };
    nextSettings.appearance.runningBubble.animationMs = clampRunningBubbleAnimationMs(nextSettings.appearance.runningBubble.animationMs);
    nextSettings.appearance.runningBubble.borderWidth = clampRunningBubbleBorderWidth(nextSettings.appearance.runningBubble.borderWidth);
    nextSettings.pet.imagePixelSize = clampImagePixelSize(nextSettings.pet.imagePixelSize);
    nextSettings.pet.opacity = clampPetOpacity(nextSettings.pet.opacity ?? defaultPetOpacity);
    nextSettings.pet.whipReactionSound = nextSettings.pet.whipReactionSound ?? "none";
    nextSettings.pet.customWhipReactionSoundPath = nextSettings.pet.customWhipReactionSoundPath ?? null;
    nextSettings.activityFilters = normalizeActivityFilters(nextSettings.activityFilters);
    nextSettings.agents = normalizeAgentSettings(nextSettings.agents);
    return nextSettings;
  }

  function normalizeActivityFilters(filters: Partial<AppSettings["activityFilters"]> | null | undefined): AppSettings["activityFilters"] {
    const legacyFilters = normalizeAgentActivityFilters(filters);
    const rawByAgent = filters?.byAgent ?? {};
    const hasAgentFilters = Object.keys(rawByAgent).length > 0;
    const byAgent = agentOrder.reduce<AppSettings["activityFilters"]["byAgent"]>((nextByAgent, agentId) => {
      nextByAgent[agentId] = normalizeAgentActivityFilters(rawByAgent[agentId] ?? (!hasAgentFilters ? legacyFilters : undefined));
      return nextByAgent;
    }, {});

    return {
      titleKeywords: [],
      messageKeywords: [],
      byAgent,
    };
  }

  function normalizeAgentActivityFilters(filters: Partial<ActivityKeywordFilterSettings> | null | undefined): ActivityKeywordFilterSettings {
    return {
      titleKeywords: normalizeFilterKeywords(filters?.titleKeywords),
      messageKeywords: normalizeFilterKeywords(filters?.messageKeywords),
    };
  }

  function normalizeAgentSettings(agentSettings: Partial<AppSettings["agents"]> | null | undefined): AppSettings["agents"] {
    const rawByAgent = agentSettings?.byAgent ?? {};
    const byAgent = agents.reduce<AppSettings["agents"]["byAgent"]>((nextByAgent, agent) => {
      const configured = rawByAgent[agent.id]?.hookEvents ?? agent.selectedHookEvents;
      nextByAgent[agent.id] = {
        hookEvents: normalizeHookEventsForAgent(agent, configured),
      };
      return nextByAgent;
    }, { ...agentSettingsDefaults.byAgent });
    return { byAgent };
  }

  function normalizeHookEventsForAgent(agent: AgentView, hookEvents: string[] | null | undefined) {
    const selected = agent.hookEvents.filter((event) => hookEvents?.includes(event));
    return selected.length ? selected : [...agent.hookEvents];
  }

  function normalizeFilterKeywords(keywords: string[] | null | undefined): string[] {
    const seen = new Set<string>();
    const normalized: string[] = [];
    for (const keyword of keywords ?? []) {
      const value = keyword.trim();
      const key = value.toLocaleLowerCase();
      if (!value || seen.has(key)) {
        continue;
      }
      seen.add(key);
      normalized.push(value);
    }
    return normalized;
  }

  function createFilterDrafts(): Record<AgentId, Record<ActivityFilterKind, string>> {
    return agentOrder.reduce<Record<AgentId, Record<ActivityFilterKind, string>>>((drafts, agentId) => {
      drafts[agentId] = {
        titleKeywords: "",
        messageKeywords: "",
      };
      return drafts;
    }, {} as Record<AgentId, Record<ActivityFilterKind, string>>);
  }

  function agentActivityFilters(agentId: AgentId): ActivityKeywordFilterSettings {
    return settings?.activityFilters.byAgent?.[agentId] ?? agentActivityFilterDefaults;
  }

  function updateFilterDraft(agentId: AgentId, kind: ActivityFilterKind, value: string) {
    filterDrafts = {
      ...filterDrafts,
      [agentId]: {
        ...filterDrafts[agentId],
        [kind]: value,
      },
    };
  }

  async function addFilterKeyword(agentId: AgentId, kind: ActivityFilterKind) {
    if (!settings) return;
    const value = filterDrafts[agentId][kind].trim();
    if (!value) {
      return;
    }
    await updateActivityFilterKeywords(agentId, kind, [...agentActivityFilters(agentId)[kind], value]);
    updateFilterDraft(agentId, kind, "");
  }

  async function removeFilterKeyword(agentId: AgentId, kind: ActivityFilterKind, keyword: string) {
    if (!settings) return;
    const target = keyword.toLocaleLowerCase();
    await updateActivityFilterKeywords(agentId, kind, agentActivityFilters(agentId)[kind].filter((item) => item.toLocaleLowerCase() !== target));
  }

  async function updateActivityFilterKeywords(agentId: AgentId, kind: ActivityFilterKind, keywords: string[]) {
    if (!settings) return;
    const agentFilters = {
      ...agentActivityFilterDefaults,
      ...agentActivityFilters(agentId),
      [kind]: normalizeFilterKeywords(keywords),
    };
    settings = {
      ...settings,
      activityFilters: {
        ...activityFilterDefaults,
        ...settings.activityFilters,
        titleKeywords: [],
        messageKeywords: [],
        byAgent: {
          ...settings.activityFilters.byAgent,
          [agentId]: agentFilters,
        },
      },
    };
    await saveSettings();
  }

  function handleFilterDraftKeydown(event: KeyboardEvent, agentId: AgentId, kind: ActivityFilterKind) {
    if (event.key !== "Enter") {
      return;
    }
    event.preventDefault();
    void addFilterKeyword(agentId, kind);
  }

  async function clearAgentActivityFilters(agentId: AgentId) {
    if (!settings) return;
    settings = {
      ...settings,
      activityFilters: {
        ...activityFilterDefaults,
        ...settings.activityFilters,
        titleKeywords: [],
        messageKeywords: [],
        byAgent: {
          ...settings.activityFilters.byAgent,
          [agentId]: { ...agentActivityFilterDefaults },
        },
      },
    };
    await saveSettings();
  }

  function agentActivityFilterCount(filters: ActivityKeywordFilterSettings | null | undefined) {
    return normalizeFilterKeywords(filters?.titleKeywords).length + normalizeFilterKeywords(filters?.messageKeywords).length;
  }

  function clampImagePixelSize(value: number) {
    return Math.min(128, Math.max(16, Math.round(value || defaultImagePixelSize)));
  }

  function imagePixelSizeLabel(value: number) {
    const pixelSize = clampImagePixelSize(value);
    return `${pixelSize}px`;
  }

  function clampPetOpacity(value: number | null | undefined) {
    const numericValue = Number(value);
    if (!Number.isFinite(numericValue)) {
      return defaultPetOpacity;
    }
    return Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue));
  }

  function petOpacityLabel(value: number | null | undefined) {
    return `${Math.round(clampPetOpacity(value) * 100)}%`;
  }

  function clampRunningBubbleAnimationMs(value: number) {
    return Math.min(4000, Math.max(600, Math.round(value || runningBubbleDefaults.animationMs)));
  }

  function runningBubbleSpeedLabel(value: number) {
    return `${(clampRunningBubbleAnimationMs(value) / 1000).toFixed(1)}s`;
  }

  function clampRunningBubbleBorderWidth(value: number) {
    return Math.min(8, Math.max(1, Math.round(value || runningBubbleDefaults.borderWidth)));
  }

  function runningBubbleBorderWidthLabel(value: number) {
    return `${clampRunningBubbleBorderWidth(value)}px`;
  }

  function bubbleColorEditor(value: string | null | undefined, fallback: string) {
    return gradientEditorFromCss(value, fallback);
  }

  function currentBubbleColorEditor(key: RunningBubbleColorKey, fallback: string) {
    return bubbleColorEditor(settings?.appearance.runningBubble[key], fallback);
  }

  function updateBubbleColor(
    key: RunningBubbleColorKey,
    fallback: string,
    patch: Partial<GradientEditorValue>,
  ) {
    if (!settings) return;
    settings = updateRunningBubbleColorSetting(settings, key, fallback, patch);
    scheduleRunningBubbleSettingsSave();
  }

  function updateBubbleColorStop(
    key: RunningBubbleColorKey,
    fallback: string,
    index: number,
    color: string,
  ) {
    const editor = currentBubbleColorEditor(key, fallback);
    const colors = [...editor.colors];
    colors[index] = color;
    updateBubbleColor(key, fallback, { colors });
  }

  function setSelectedBubbleColorStop(key: RunningBubbleColorKey, index: number) {
    selectedBubbleColorStop = {
      ...selectedBubbleColorStop,
      [key]: Math.max(0, index),
    };
  }

  function addBubbleColorStop(key: RunningBubbleColorKey, fallback: string) {
    const editor = currentBubbleColorEditor(key, fallback);
    const nextColors = [...editor.colors, nextGradientStopColor(editor.colors)];
    setSelectedBubbleColorStop(key, nextColors.length - 1);
    updateBubbleColor(key, fallback, { colors: nextColors });
  }

  function removeBubbleColorStop(key: RunningBubbleColorKey, fallback: string, index: number) {
    const editor = currentBubbleColorEditor(key, fallback);
    const colors = editor.colors.filter((_, colorIndex) => colorIndex !== index);
    setSelectedBubbleColorStop(key, Math.max(0, Math.min(selectedBubbleColorStop[key], colors.length - 1)));
    updateBubbleColor(key, fallback, { colors: colors.length ? colors : [fallback] });
  }

  function selectedBubbleColorIndex(selectedIndex: number | undefined, count: number) {
    return Math.max(0, Math.min(selectedIndex ?? 0, Math.max(count - 1, 0)));
  }

  function selectPreviousBubbleColorStop(key: RunningBubbleColorKey, count: number) {
    setSelectedBubbleColorStop(key, Math.max(0, selectedBubbleColorIndex(selectedBubbleColorStop[key], count) - 1));
  }

  function selectNextBubbleColorStop(key: RunningBubbleColorKey, count: number) {
    setSelectedBubbleColorStop(key, Math.min(Math.max(count - 1, 0), selectedBubbleColorIndex(selectedBubbleColorStop[key], count) + 1));
  }

  function selectBubbleColorStopFromBand(key: RunningBubbleColorKey, count: number, event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    setSelectedBubbleColorStop(key, colorStopIndexFromBand(count, event.clientX - rect.left, rect.width));
  }

  function inputValue(event: Event) {
    return (event.currentTarget as HTMLInputElement).value;
  }

  function inputNumber(event: Event) {
    return Number((event.currentTarget as HTMLInputElement).value);
  }

  function scheduleRunningBubbleSettingsSave() {
    runningBubbleSaveToken += 1;
    clearRunningBubbleSaveTimer();
    runningBubbleSaveTimer = window.setTimeout(() => {
      runningBubbleSaveTimer = null;
      void saveRunningBubbleSettings();
    }, 250);
  }

  function clearRunningBubbleSaveTimer() {
    if (runningBubbleSaveTimer) {
      window.clearTimeout(runningBubbleSaveTimer);
      runningBubbleSaveTimer = null;
    }
  }

  async function saveRunningBubbleSettings() {
    if (!settings) return;
    const saveToken = runningBubbleSaveToken;
    const snapshot: AppSettings = {
      ...settings,
      appearance: {
        ...settings.appearance,
        runningBubble: {
          ...settings.appearance.runningBubble,
          animationMs: clampRunningBubbleAnimationMs(settings.appearance.runningBubble.animationMs),
          borderWidth: clampRunningBubbleBorderWidth(settings.appearance.runningBubble.borderWidth),
        },
      },
    };

    try {
      const savedSettings = normalizeSettings(await updateAppSettings(snapshot));
      if (saveToken !== runningBubbleSaveToken || !settings) return;
      settings = {
        ...settings,
        appearance: {
          ...settings.appearance,
          runningBubble: savedSettings.appearance.runningBubble,
        },
      };
    } catch (currentError) {
      if (saveToken === runningBubbleSaveToken) {
        error = String(currentError);
      }
    }
  }

  async function savePetImagePixelSize() {
    if (!settings) return;
    settings.pet.imagePixelSize = clampImagePixelSize(settings.pet.imagePixelSize);
    busyPet = "pixel-size";
    error = "";
    try {
      petLibrary = await updatePetImagePixelSize(settings.pet.imagePixelSize);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function savePetOpacity() {
    if (!settings) return;
    settings.pet.opacity = clampPetOpacity(settings.pet.opacity);
    await saveSettings();
  }

  function statusLabel(status: PetEvent["status"]) {
    return {
      idle: "待命",
      thinking: "正在思考",
      running: "正在执行",
      "waiting-approval": "等待授权",
      failed: "任务失败",
      done: "任务完成",
    }[status];
  }

  function kindLabel(kind: PetEvent["kind"]) {
    return {
      "task-started": "任务开始",
      "task-updated": "任务更新",
      "tool-started": "工具调用",
      "permission-requested": "授权请求",
      message: "消息",
      "task-failed": "任务失败",
      "task-completed": "任务完成",
    }[kind];
  }

  function shortTime(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return "";
    return date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  }

  function soundLabel(sound: AppSettings["notifications"]["sound"]) {
    return {
      blip: "Blip",
      chime: "Chime",
      bell: "Bell",
      custom: "自定义",
      silent: "静音",
    }[sound];
  }

  function whipReactionSoundLabel(sound: AppSettings["pet"]["whipReactionSound"]) {
    return whipReactionSounds.find((option) => option.value === sound)?.label ?? "无";
  }

  function compactNumber(value: number | undefined) {
    return Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 1 }).format(value ?? 0);
  }

  function agentLabel(agentId: AgentView["id"]) {
    return {
      codex: "Codex",
      claude: "Claude Code",
      qoder: "Qoder",
      cursor: "Cursor",
    }[agentId];
  }

  function formatBucketLabel(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return value;
    return date.toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
  }

  function agentSegmentHeight(tokens: number | undefined, maxTokens: number) {
    if (!tokens || !maxTokens) return "0%";
    return `${Math.max(4, (tokens / maxTokens) * 100)}%`;
  }

  function usageFilterLabel(value: UsageRange | UsageBucketSize) {
    return [...usageRanges, ...usageBucketSizes].find((option) => option.value === value)?.label ?? value;
  }

  function usageProviderTotal(agentId: AgentView["id"]) {
    return usageData.byProvider.find((provider) => provider.provider === agentId);
  }

  $: latest = events.at(-1);
  $: recentVisibleEvents = events.slice(-5).reverse();
  $: enabledAgents = agents.filter((agent) => agent.enabled);
  $: enabledHookCount = enabledAgents.reduce((count, agent) => count + selectedHookEvents(agent).length, 0);
  $: usageData = buildUsageChartData(usage, { range: usageRange, bucketSize: usageBucketSize });
  $: usageBuckets = usageData.buckets;
  $: usageMaxTokens = usageData.maxTokens;
  $: usageTickLabels = yAxisTicks(usageMaxTokens);
  $: pageTitle = tab === "agents" ? "Agent" : tab === "usage" ? "用量" : tab === "personalize" ? "个性化" : "最新事件";
  $: appTheme = themeClassNames(settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "dark" : "light");
</script>

<main class={`app-shell pixel-shell ${appTheme}`}>
  <aside class="sidebar pixel-panel">
    <nav class="tabs" aria-label="Code Pet settings">
      <button class:active={tab === "agents"} on:click={() => (tab = "agents")} aria-label="Agent 列表">
        <Bot size={18} /> Agent
      </button>
      <button class:active={tab === "usage"} on:click={() => (tab = "usage")} aria-label="用量统计">
        <BarChart3 size={18} /> 用量
      </button>
      <button class:active={tab === "personalize"} on:click={() => (tab = "personalize")} aria-label="个性化配置">
        <Palette size={18} /> 个性化
      </button>
      <button class:active={tab === "events"} on:click={() => (tab = "events")} aria-label="最新事件">
        <Activity size={18} /> 事件
      </button>
    </nav>
  </aside>

  <section class="content">
    <header class="topbar">
      <div>
        <h2>{pageTitle}</h2>
        {#if error}<p class="error">{error}</p>{/if}
      </div>
    </header>

    {#if tab === "agents"}
      <div class="agent-workspace">
        <section class="overview-grid" aria-label="运行概览">
          <article class="overview-card pixel-panel">
            <span><PlugZap size={17} /> Hooks</span>
            <strong>{enabledHookCount}</strong>
            <p>{enabledAgents.length}/{agents.length || 0} 个 agent 已启用</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><Activity size={17} /> 最新状态</span>
            <strong>{latest ? statusLabel(latest.status) : "待命"}</strong>
            <p>{latest?.title ?? "还没有收到新的任务事件"}</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><ShieldAlert size={17} /> 授权提醒</span>
            <strong>{settings?.notifications.ringOnPermission ? "响铃" : "静音"}</strong>
            <p>{endpoint || "collector endpoint starting"}</p>
          </article>
        </section>

        <section class="agent-section pixel-panel">
          <header class="section-head">
            <div>
              <span class="agent-kicker">CONNECTED AGENTS</span>
              <h3>接入状态</h3>
            </div>
            <span>{agents.length} agents</span>
          </header>

          <div class="agent-list">
            {#each agents as agent}
              <article class="agent-card">
                <div class="agent-title">
                  <span class="agent-kicker">{agent.id}</span>
                  <h3>{agent.name}</h3>
                  <p class="agent-description">{agent.description}</p>
                </div>
                <dl class="agent-meta">
                  <div>
                    <dt>配置</dt>
                    <dd>{agent.configPath}</dd>
                  </div>
                  <div>
                    <dt>事件</dt>
                    <dd>{selectedHookEvents(agent).length}/{agent.hookEvents.length} 个 hooks</dd>
                  </div>
                </dl>
                <div class="event-row">
                  {#each agent.hookEvents as hookEvent}
                    <label class="hook-event-check" class:active={hookEventSelected(agent, hookEvent)}>
                      <input
                        type="checkbox"
                        checked={hookEventSelected(agent, hookEvent)}
                        disabled={agentBusy(agent) || isLastSelectedHookEvent(agent, hookEvent)}
                        on:change={(event) => toggleHookEvent(agent, hookEvent, event.currentTarget.checked)}
                      />
                      <span>{hookEvent}</span>
                    </label>
                  {/each}
                </div>
                {#if settings}
                  <div class="agent-filter-panel">
                    <div class="filter-card-head compact">
                      <h3><Filter size={16} /> 任务过滤</h3>
                      <button class="filter-clear-button" disabled={agentActivityFilterCount(agentActivityFilters(agent.id)) === 0} on:click={() => clearAgentActivityFilters(agent.id)}>
                        <Trash2 size={15} /> 清空
                      </button>
                    </div>
                    <div class="compact-filter-groups">
                      {#each activityFilterGroups as group}
                        <div class="compact-filter-group">
                          <span>{group.label}</span>
                          <div class="filter-chip-row">
                            {#each agentActivityFilters(agent.id)[group.key] as keyword}
                              <button class="filter-chip" type="button" on:click={() => removeFilterKeyword(agent.id, group.key, keyword)} aria-label={`移除${agent.name} ${group.label}过滤 ${keyword}`}>
                                {keyword}
                                <Trash2 size={12} />
                              </button>
                            {/each}
                            <input
                              value={filterDrafts[agent.id][group.key]}
                              placeholder={group.placeholder}
                              on:input={(event) => updateFilterDraft(agent.id, group.key, event.currentTarget.value)}
                              on:keydown={(event) => handleFilterDraftKeydown(event, agent.id, group.key)}
                            />
                            <button class="filter-add-button" type="button" on:click={() => addFilterKeyword(agent.id, group.key)}>添加</button>
                          </div>
                        </div>
                      {/each}
                    </div>
                  </div>
                {/if}
                <div class="agent-controls">
                  <span class:online={agent.enabled} class="status-chip">{agent.enabled ? "已启用" : "未启用"}</span>
                  <button
                    class:enabled={agent.enabled}
                    class="power-button"
                    disabled={agentBusy(agent)}
                    on:click={() => toggleAgent(agent)}
                    aria-label={`${agent.name} ${agent.enabled ? "关闭" : "启用"}`}
                  >
                    <Power size={17} />
                  </button>
                </div>
              </article>
            {/each}
          </div>
        </section>
      </div>
    {:else if tab === "usage"}
      <div class="usage-workspace">
        <section class="usage-summary-grid" aria-label="Token 用量概览">
          <article class="overview-card pixel-panel">
            <span><BarChart3 size={17} /> 总量</span>
            <strong>{compactNumber(usageData.total.totalTokens)}</strong>
            <p>{usageFilterLabel(usageRange)} · 输入 {compactNumber(usageData.total.inputTokens)} · 输出 {compactNumber(usageData.total.outputTokens)}</p>
          </article>
          {#each agentOrder as agentId}
            {@const provider = usageProviderTotal(agentId)}
            <article class="overview-card pixel-panel">
              <span>{agentLabel(agentId)}</span>
              <strong>{compactNumber(provider?.total.totalTokens)}</strong>
              <p>输入 {compactNumber(provider?.total.inputTokens)} · 输出 {compactNumber(provider?.total.outputTokens)}</p>
            </article>
          {/each}
        </section>

        <section class="usage-panel pixel-panel">
          <header class="section-head">
            <div>
              <span class="agent-kicker">{usageFilterLabel(usageBucketSize)} / {usageFilterLabel(usageRange)}</span>
              <h3>Token 用量</h3>
            </div>
            <div class="usage-controls" aria-label="用量统计设置">
              <label>
                范围
                <select bind:value={usageRange}>
                  {#each usageRanges as range}
                    <option value={range.value}>{range.label}</option>
                  {/each}
                </select>
              </label>
              <label>
                单位
                <select bind:value={usageBucketSize}>
                  {#each usageBucketSizes as bucketSize}
                    <option value={bucketSize.value}>{bucketSize.label}</option>
                  {/each}
                </select>
              </label>
            </div>
          </header>

          {#if usageBuckets.length}
            <div class="usage-chart-frame" aria-label="按 Agent 和时间单位聚合的 token 用量柱状图">
              <div class="usage-y-axis" aria-hidden="true">
                {#each usageTickLabels as tick}
                  <span>{compactNumber(tick)}</span>
                {/each}
              </div>
              <div class="usage-chart">
                {#each usageBuckets as bucket}
                  <div class="usage-column">
                    <button class="usage-bar" type="button" aria-label={`${formatBucketLabel(bucket.bucketStart)} ${compactNumber(bucket.total.totalTokens)} tokens`}>
                      {#each agentOrder as agentId}
                        {@const agentUsage = bucket.agents[agentId]}
                        {#if agentUsage && agentUsage.totalTokens > 0}
                          <span
                            class={`usage-segment ${agentId}`}
                            style={`height: ${agentSegmentHeight(agentUsage.totalTokens, usageMaxTokens)}`}
                            aria-label={`${agentLabel(agentId)} ${compactNumber(agentUsage.totalTokens)} tokens`}
                          ></span>
                        {/if}
                      {/each}
                      <span class="usage-tooltip">
                        <strong>{formatBucketLabel(bucket.bucketStart)}</strong>
                        <em>总量 {compactNumber(bucket.total.totalTokens)} · 输入 {compactNumber(bucket.total.inputTokens)} · 输出 {compactNumber(bucket.total.outputTokens)}</em>
                        {#each agentOrder as agentId}
                          {@const agentUsage = bucket.agents[agentId]}
                          {#if agentUsage && agentUsage.totalTokens > 0}
                            <span><i class={`usage-dot ${agentId}`}></i>{agentLabel(agentId)} {compactNumber(agentUsage.totalTokens)} · 输入 {compactNumber(agentUsage.inputTokens)} · 输出 {compactNumber(agentUsage.outputTokens)}</span>
                          {/if}
                        {/each}
                      </span>
                    </button>
                    <span class="usage-axis-label">{formatBucketLabel(bucket.bucketStart)}</span>
                  </div>
                {/each}
              </div>
            </div>
            <div class="usage-legend" aria-label="Agent 图例">
              <span><i class="usage-dot codex"></i> Codex</span>
              <span><i class="usage-dot claude"></i> Claude Code</span>
              <span><i class="usage-dot qoder"></i> Qoder</span>
              <span><i class="usage-dot cursor"></i> Cursor</span>
            </div>
          {:else}
            <div class="empty-state">
              <BarChart3 size={20} />
              <strong>还没有用量数据</strong>
              <p>收到 Codex、Claude Code、Qoder 或 Cursor 的 transcript 后，这里会按选择的时间范围展示 token 用量。</p>
            </div>
          {/if}
        </section>

        {#if usageBuckets.length}
          <section class="usage-table pixel-panel">
            <header class="section-head">
              <div>
                <h3>明细</h3>
              </div>
            </header>
            {#each usageBuckets.slice().reverse() as bucket}
              <div class="usage-row">
                <span>{formatBucketLabel(bucket.bucketStart)}</span>
                <strong>{compactNumber(bucket.total.totalTokens)}</strong>
                <em>Codex {compactNumber(bucket.agents.codex?.totalTokens)} · Claude {compactNumber(bucket.agents.claude?.totalTokens)} · Qoder {compactNumber(bucket.agents.qoder?.totalTokens)} · Cursor {compactNumber(bucket.agents.cursor?.totalTokens)}</em>
              </div>
            {/each}
          </section>
        {/if}
      </div>
    {:else if tab === "personalize" && settings}
      <div class="personal-grid">
        <section class="pet-editor pixel-panel">
          <header class="panel-head">
            <div>
              <h3>像素形象</h3>
            </div>
          </header>
          <div class="pet-preview codex-preview">
            <PetAvatar sprite={settings.pet.sprite} kind={settings.pet.kind} imagePath={settings.pet.imagePath} status={latest?.status ?? "thinking"} scale={Math.max(settings.pet.scale, 4)} />
            <button class="preview-import-button" disabled={busyPet === "import" || busyPet === "cutout-import"} on:click={() => importImagePet(true)} aria-label="抠图导入图片宠物">
              <ImagePlus size={18} />
            </button>
          </div>
          <label class="image-pixel-control">
            <span>
              <span>像素化程度</span>
              <strong>{imagePixelSizeLabel(settings.pet.imagePixelSize)}</strong>
            </span>
            <input
              type="range"
              min="16"
              max="128"
              step="8"
              bind:value={settings.pet.imagePixelSize}
              disabled={busyPet === "pixel-size"}
              on:change={savePetImagePixelSize}
            />
          </label>
          <label class="pet-opacity-control">
            <span>
              <span>窗口不透明度</span>
              <strong>{petOpacityLabel(settings.pet.opacity)}</strong>
            </span>
            <input
              type="range"
              min={minPetOpacity}
              max="1"
              step="0.05"
              bind:value={settings.pet.opacity}
              on:input={(event) => (settings.pet.opacity = clampPetOpacity(inputNumber(event)))}
              on:change={savePetOpacity}
            />
          </label>
          <section class="pet-library-panel">
            <div class="panel-head compact">
              <div>
                <h3>宠物库</h3>
              </div>
            </div>
            <div class="data-directory">
              <span>{petLibrary?.dataDirectory ?? settings.petLibrary.dataDirectory ?? "app data/code-pet/pets"}</span>
              <button disabled={busyPet === "directory"} on:click={choosePetDataDirectory}>
                <FolderCog size={16} /> 修改
              </button>
            </div>
            <div class="pet-list" aria-label="已配置宠物">
              {#each petLibrary?.pets ?? settings.petLibrary.pets as pet}
                {@const isActivePet = (petLibrary?.selectedPetId ?? settings.pet.selectedPetId) === pet.id}
                <article class="pet-item" class:active={isActivePet}>
                  <button class="pet-select-button" disabled={busyPet === pet.id} on:click={() => activatePet(pet.id)}>
                    <span class="pet-thumb">
                      <PetAvatar
                        sprite={pet.sprite ?? settings.pet.sprite}
                        kind={pet.kind}
                        imagePath={pet.imagePath}
                        status="idle"
                        scale={2}
                        label={pet.name}
                      />
                    </span>
                    <span>
                      <strong>{pet.name}</strong>
                      <em>{pet.kind === "codex-atlas" ? "Codex 宠物" : pet.kind === "image" ? "导入图片" : "调色板"}</em>
                    </span>
                    {#if isActivePet}
                      <Check size={17} />
                    {/if}
                  </button>
                  {#if pet.id !== "default"}
                    <button
                      class="pet-delete-button"
                      disabled={busyPet === `delete:${pet.id}`}
                      on:click={(event) => removePet(event, pet.id)}
                      aria-label={`删除 ${pet.name}`}
                    >
                      <Trash2 size={16} />
                    </button>
                  {/if}
                </article>
              {/each}
            </div>
          </section>
        </section>

        <div class="personal-side">
          <section class="appearance-editor pixel-panel">
            <header class="panel-head">
              <h3>主题</h3>
            </header>
            <section class="theme-switcher" aria-label="主题模式">
              <button class:active={settings.appearance.theme === "light"} on:click={() => setTheme("light")} aria-label="浅色模式">
                <Sun size={16} /> Light
              </button>
              <button class:active={settings.appearance.theme === "dark"} on:click={() => setTheme("dark")} aria-label="深色模式">
                <Moon size={16} /> Dark
              </button>
              <button class:active={settings.appearance.theme === "system"} on:click={() => setTheme("system")} aria-label="跟随系统">
                Auto
              </button>
            </section>
          </section>

          <section class="bubble-editor pixel-panel">
            <header class="panel-head">
              <h3>任务气泡</h3>
            </header>
            <div class="bubble-toggle-grid">
              <label class="check">
                <input type="checkbox" bind:checked={settings.appearance.runningBubble.backgroundBreathing} on:change={saveRunningBubbleSettings} />
                背景色呼吸灯
              </label>
              <label class="check">
                <input type="checkbox" bind:checked={settings.appearance.runningBubble.borderMarquee} on:change={saveRunningBubbleSettings} />
                边框跑马灯
              </label>
            </div>
            <div class="bubble-color-grid">
              {#each bubbleColorConfigs as colorConfig}
                {@const editor = bubbleColorEditor(settings.appearance.runningBubble[colorConfig.key], colorConfig.fallback)}
                {@const selectedColorIndex = selectedBubbleColorIndex(selectedBubbleColorStop[colorConfig.key], editor.colors.length)}
                <section class="gradient-editor" aria-label={colorConfig.label}>
                  <div class="gradient-editor-head">
                    <strong>{colorConfig.label}</strong>
                  </div>
                  <button
                    class="color-band-preview"
                    type="button"
                    aria-label={`选择${colorConfig.label}色段`}
                    style={`background: ${gradientSegmentCss(editor.colors)}`}
                    on:click={(event) => selectBubbleColorStopFromBand(colorConfig.key, editor.colors.length, event)}
                  ></button>
                  <div class="color-stop-editor" aria-label={`${colorConfig.label}当前颜色`}>
                    <button type="button" aria-label="上一个颜色" disabled={selectedColorIndex === 0} on:click={() => selectPreviousBubbleColorStop(colorConfig.key, editor.colors.length)}>‹</button>
                    <label class="color-stop">
                      {#key `${colorConfig.key}-${selectedColorIndex}-${editor.colors[selectedColorIndex]}`}
                        <input type="color" value={editor.colors[selectedColorIndex]} on:input={(event) => updateBubbleColorStop(colorConfig.key, colorConfig.fallback, selectedColorIndex, inputValue(event))} />
                      {/key}
                    </label>
                    <button type="button" aria-label="下一个颜色" disabled={selectedColorIndex >= editor.colors.length - 1} on:click={() => selectNextBubbleColorStop(colorConfig.key, editor.colors.length)}>›</button>
                    <span>{selectedColorIndex + 1}/{editor.colors.length}</span>
                    <button class="add-color-stop" type="button" on:click={() => addBubbleColorStop(colorConfig.key, colorConfig.fallback)}>添加颜色</button>
                    {#if editor.colors.length > 1}
                      <button class="remove-color-stop" type="button" aria-label={`移除当前 ${colorConfig.label}颜色`} on:click={() => removeBubbleColorStop(colorConfig.key, colorConfig.fallback, selectedColorIndex)}>移除</button>
                    {/if}
                  </div>
                  {#if colorConfig.directional}
                    <label>
                      角度 <strong>{editor.angle}deg</strong>
                      <input type="range" min="0" max="360" step="5" value={editor.angle} on:input={(event) => updateBubbleColor(colorConfig.key, colorConfig.fallback, { angle: inputNumber(event) })} />
                    </label>
                  {/if}
                </section>
              {/each}
            </div>
            <label>
              边框宽度 <strong>{runningBubbleBorderWidthLabel(settings.appearance.runningBubble.borderWidth)}</strong>
              <input
                type="range"
                min="1"
                max="8"
                step="1"
                bind:value={settings.appearance.runningBubble.borderWidth}
                on:change={saveRunningBubbleSettings}
              />
            </label>
            <label>
              动画速率 <strong>{runningBubbleSpeedLabel(settings.appearance.runningBubble.animationMs)}</strong>
              <input
                type="range"
                min="600"
                max="4000"
                step="100"
                bind:value={settings.appearance.runningBubble.animationMs}
                on:change={saveRunningBubbleSettings}
              />
            </label>
          </section>

          <section class="appearance-editor pixel-panel">
            <header class="panel-head">
              <h3><Rocket size={18} /> 系统</h3>
            </header>
            <div class="system-data-directory">
              <div class="setting-line">
                <span>数据目录</span>
                <em>{settings.data.dataDirectory ? "自定义" : "默认"}</em>
              </div>
              <div class="data-directory">
                <span>{appDataDir || settings.data.dataDirectory || "app data/code-pet"}</span>
                <div class="directory-actions">
                  <button disabled={busyAppDataDirectory} on:click={chooseAppDataDirectory}>
                    <FolderCog size={16} /> 修改
                  </button>
                  <button disabled={busyAppDataDirectory || !settings.data.dataDirectory} on:click={resetAppDataDirectory} aria-label="恢复默认数据目录">
                    <RotateCcw size={16} /> 默认
                  </button>
                </div>
              </div>
              <p class="setting-note">
                {appDataRestartPending ? "已保存并复制原数据，重启后完全生效。" : "修改后会复制原数据，保存完成后请重启。"}
              </p>
            </div>
            <label class="check">
              <input type="checkbox" checked={launchAtLogin} disabled={busyLaunchAtLogin} on:change={toggleLaunchAtLogin} />
              开机自启动
            </label>
          </section>

          <section class="sound-editor pixel-panel">
            <header class="panel-head">
              <div>
                <h3><Bell size={18} /> 通知声音</h3>
              </div>
            </header>
            <div class="sound-summary">
              <strong>{soundLabel(settings.notifications.sound)}</strong>
              <span>{settings.notifications.ringOnPermission ? "授权时会响铃" : "授权提醒静音"} · {settings.notifications.ringOnFailure ? "失败时会响铃" : "失败提醒静音"} · {settings.notifications.ringOnDone ? "结束时会响铃" : "结束提醒静音"}</span>
            </div>
            <div class="segmented">
              {#each ["blip", "chime", "bell", "custom", "silent"] as sound}
                <button
                  class:active={settings.notifications.sound === sound}
                  on:click={async () => {
                    settings.notifications.sound = sound as AppSettings["notifications"]["sound"];
                    await saveSettings();
                  }}
                >
                  {sound}
                </button>
              {/each}
            </div>
            <div class="row-actions">
              <button on:click={() => playNotificationSound(settings)}>
                <Volume2 size={17} /> 试听
              </button>
              <button on:click={pickCustomSound}>
                <FolderOpen size={17} /> 选择音频
              </button>
            </div>
            {#if settings.notifications.customSoundPath}
              <p class="path">{settings.notifications.customSoundPath}</p>
            {/if}
            <div class="sound-subsection">
              <strong>抽打反应</strong>
              <span>抽完鞭子后，桌宠继续发出的声音：{whipReactionSoundLabel(settings.pet.whipReactionSound)}</span>
            </div>
            <div class="segmented">
              {#each whipReactionSounds as reaction}
                <button
                  class:active={settings.pet.whipReactionSound === reaction.value}
                  on:click={async () => {
                    settings.pet.whipReactionSound = reaction.value;
                    await saveSettings();
                  }}
                >
                  {reaction.label}
                </button>
              {/each}
            </div>
            <div class="row-actions">
              <button on:click={() => playWhipReactionSound(settings.pet.whipReactionSound, settings.pet.customWhipReactionSoundPath)}>
                <Volume2 size={17} /> 试听反应
              </button>
              <button on:click={pickCustomWhipReactionSound}>
                <FolderOpen size={17} /> 选择反应音频
              </button>
            </div>
            {#if settings.pet.customWhipReactionSoundPath}
              <p class="path">{settings.pet.customWhipReactionSoundPath}</p>
            {/if}
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.ringOnPermission} on:change={saveSettings} />
              授权时响铃
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.ringOnFailure} on:change={saveSettings} />
              失败时响铃
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.ringOnDone} on:change={saveSettings} />
              任务结束时响铃
            </label>
            <label>
              重复提醒
              <input type="number" min="5" max="300" bind:value={settings.notifications.repeatSeconds} on:change={saveSettings} />
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.quietHoursEnabled} on:change={saveSettings} />
              静音时段
            </label>
            <div class="time-row">
              <input type="time" bind:value={settings.notifications.quietHoursStart} on:change={saveSettings} />
              <input type="time" bind:value={settings.notifications.quietHoursEnd} on:change={saveSettings} />
            </div>
          </section>
        </div>
      </div>
    {:else if tab === "events"}
      <section class="event-log pixel-panel">
        <header class="section-head">
          <span>{events.length} total</span>
        </header>
        {#if recentVisibleEvents.length}
          {#each recentVisibleEvents as event}
            <div class="event-item">
              <span class="event-provider">{event.provider}</span>
              <div>
                <strong>{event.title}</strong>
                <p>{event.message}</p>
              </div>
              <span class="event-kind">{kindLabel(event.kind)}</span>
              <span class="event-time"><Clock3 size={14} /> {shortTime(event.createdAt)}</span>
            </div>
          {/each}
        {:else}
          <div class="empty-state">
            <Activity size={20} />
            <strong>还没有事件</strong>
            <p>启动 Codex、Claude Code 或 Qoder 任务后，这里会显示最近的 hooks 消息。</p>
          </div>
        {/if}
      </section>
    {/if}
  </section>
</main>
