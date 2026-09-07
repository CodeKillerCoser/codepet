<script lang="ts">
  import EventJournal from "./lib/EventJournal.svelte";
  import WindowToolbar from "./lib/WindowToolbar.svelte";
  import { basename, extname, join } from "@tauri-apps/api/path";
  import PetSources from "./lib/PetSources.svelte";
  import type { PetSource } from "./lib/petGateway";
  import { confirm as confirmDialog, open } from "@tauri-apps/plugin-dialog";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    Activity,
    BarChart3,
    Bell,
    Bot,
    Cable,
    Check,
    Clock3,
    Cpu,
    Download,
    Filter,
    FolderCog,
    FolderOpen,
    ImagePlus,
    Moon,
    Palette,
    PlugZap,
    Plus,
    Power,
    RefreshCw,
    RotateCcw,
    Rocket,
    ShieldAlert,
    Sun,
    Trash2,
    X,
    Volume2,
  } from "@lucide/svelte";
  import { onMount, tick } from "svelte";
  import ProviderConnectionStatus from "./lib/ProviderConnectionStatus.svelte";
  import { observeProviderRuntimes, type ProviderConnectionState } from "./lib/providerRuntimes";
  import { appDataDirectory, appDataDirectoryTargetStatus, checkAppUpdate, clearAgentRuntimeExecutable, cutOutImageSubject, deletePet, detectAgentRuntime, getAppSettings, getLaunchAtLoginEnabled, importPetImage, installAppUpdate, listPets, recentEvents, recordPerfEvent, refreshAgentRuntimes, selectPet, sendTestRobotNotification, setAgentRuntimeExecutable, setAppDataDirectory, setLaunchAtLoginEnabled, setPetDataDirectory, tokenUsageSummary, updateAppSettings, updatePetImagePixelSize } from "./lib/api";
  import { agentRuntimeSourceLabel, agentRuntimeStatusMeta, canRestoreAutomaticDetection } from "./lib/agentRuntime";
  import { colorStopIndexFromBand, updateRunningBubbleColorSetting, type RunningBubbleColorKey } from "./lib/bubbleColorSettings";
  import { mergeEventFeed } from "./lib/eventFeed";
  import { gradientEditorFromCss, gradientSegmentCss, nextGradientStopColor, type GradientEditorValue } from "./lib/gradientColor";
  import PetAvatar from "./lib/PetAvatar.svelte";
  import PairDeviceDialog from "./lib/PairDeviceDialog.svelte";
  import RemoteDeviceList from "./lib/RemoteDeviceList.svelte";
  import { cancelRemotePairing, copyRemotePairingJson, getRemoteAccessStatus, getRemotePairingStatus, listRemoteClients, listRemotePairingRequests, remoteCommandDiagnostic, resolveRemotePairingRequest, retryRemoteAccess, revokeRemoteCredential, startRemotePairing, type RemoteAccessDiagnostic, type RemoteAccessStatus } from "./lib/remoteAccess";
  import { pairingJsonCanBeCopied, pairingPhaseForStatus, pairingRemainingSeconds, remoteDeviceFromClient, type PairingCopyStatus, type PairingDisplayState, type RemoteDevice } from "./lib/remoteDevices";
  import { playNotificationSound, playWhipReactionSound } from "./lib/sound";
  import { defaultRunningBubbleSettings, themeClassNames } from "./lib/theme";
  import { ignoredUpdateSettings, shouldPromptForUpdate, type UpdateCheckMode } from "./lib/updates";
  import { buildUsageChartData, yAxisTicks, type UsageBucketSize, type UsageRange } from "./lib/usageChart";
  import type { ActivityKeywordFilterSettings, AgentId, AgentRuntime, AgentRuntimeProviderId, AgentView, AppSettings, AppUpdate, DingTalkRobotChannel, PetEvent, PetLibraryView, RobotNotificationChannel, TokenUsageSummary } from "./lib/types";

  type ActivityFilterKind = keyof ActivityKeywordFilterSettings;

  let tab: "agents" | "connections" | "usage" | "personalize" | "events" = "agents";
  let sidebarCollapsed = false;
  let petSources: PetSource[] = [];
  let agentRuntimes: AgentRuntime[] = [];
  let providerConnections: ProviderConnectionState[] = [];
  let providerConnectionsLoaded = false;
  const runtimeObserver = observeProviderRuntimes({
    connections: (states) => { providerConnections = states; providerConnectionsLoaded = true; },
    runtimes: (runtimes) => { agentRuntimes = runtimes; },
    error: (currentError) => { error = String(currentError); },
  });
  let remoteDevices: RemoteDevice[] = [];
  let pairDeviceDialogOpen = false;
  let addDeviceButton: HTMLButtonElement | null = null;
  let pairingDisplay: PairingDisplayState = {
    phase: "unavailable",
    qrImageUrl: null,
    expiresAtMs: null,
    remainingSeconds: null,
    pairedClientName: null,
    errorMessage: null,
  };
  let remoteRuntimeStatus: RemoteAccessStatus | null = null;
  let remoteCommandError: RemoteAccessDiagnostic | null = null;
  let visibleRemoteDiagnostic: RemoteAccessDiagnostic | null = null;
  let remoteClientsLoading = true;
  let remoteClientsUnavailable = false;
  let remoteRefreshInFlight = false;
  let remoteRetryBusy = false;
  let revokingRemoteCredentialId: string | null = null;
  let remoteDevicesNowMs = Date.now();
  let activePairingId: string | null = null;
  let pairingCopyStatus: PairingCopyStatus = "idle";
  let pairingCopyMessage: string | null = null;
  let pairingRequestToken = 0;
  let pairingKnownDeviceIds = new Set<string>();
  let pairingPollTimer: number | null = null;
  let pairingCountdownTimer: number | null = null;
  let settings: AppSettings | null = null;
  let petLibrary: PetLibraryView | null = null;
  let usage: TokenUsageSummary | null = null;
  let events: PetEvent[] = [];
  let appDataDir = "";
  let busyRuntime: string | null = null;
  let busyPet = "";
  let busyAppDataDirectory = false;
  let appDataRestartPending = false;
  let busyLaunchAtLogin = false;
  let busyRobotChannel = "";
  let error = "";
  let robotNotificationResult = "";
  let launchAtLogin = false;
  let systemDark = false;
  let eventPollTimer: number | null = null;
  let updatePollTimer: number | null = null;
  let remoteAccessPollTimer: number | null = null;
  let remotePairingRequestPollTimer: number | null = null;
  let remotePairingRequestPollBusy = false;
  const handledRemotePairingRequestIds = new Set<string>();
  let remoteDeviceClockTimer: number | null = null;
  let updateCheckMode: UpdateCheckMode | null = null;
  let updatePromptMode: UpdateCheckMode = "auto";
  let availableUpdate: AppUpdate | null = null;
  let updateInstallBusy = false;
  let updateMessage = "";
  let updateError = "";
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
  const agentRuntimeSettingsDefaults: AppSettings["agentRuntimes"] = {
    byProvider: {},
  };
  const robotNotificationDefaults: AppSettings["notifications"]["robot"] = {
    enabled: false,
    triggers: {
      waitingApproval: true,
      taskFailed: true,
      taskDone: true,
    },
    template: {
      title: "{{statusIcon}} Code Pet | {{status}}",
      header: "{{statusIcon}} {{status}} · {{agent}}",
      primary: "**{{task}}**\n{{contentBlock}}",
      secondary: "{{cwdLine}}\n{{toolLine}}\n{{sessionLine}}",
      footer: "{{time}}",
    },
    channels: [],
  };
  const robotTemplateFields = [
    { key: "title", label: "标题", rows: 1 },
    { key: "header", label: "摘要", rows: 1 },
    { key: "primary", label: "一级内容", rows: 3 },
    { key: "secondary", label: "二级内容", rows: 3 },
    { key: "footer", label: "页脚", rows: 1 },
  ] as const;
  const robotTemplateVariables = "{{statusIcon}} {{status}} {{agent}} {{task}} {{contentBlock}} {{cwdLine}} {{toolLine}} {{sessionLine}} {{time}}";
  const robotTriggerOptions = [
    { key: "waitingApproval", label: "等待审批或输入" },
    { key: "taskFailed", label: "任务失败" },
    { key: "taskDone", label: "任务完成" },
  ] as const;
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
  const updateAutoIntervalMs = 6 * 60 * 60 * 1000;
  const remoteAccessPollIntervalMs = 10_000;
  const remotePairingRequestPollIntervalMs = 2_000;

  $: visibleRemoteDiagnostic = remoteRuntimeStatus?.diagnostic ?? remoteCommandError;

  onMount(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const syncTheme = () => {
      systemDark = media.matches;
    };
    media.addEventListener("change", syncTheme);
    void refreshRemoteAccess();
    remoteAccessPollTimer = window.setInterval(() => {
      if (tab === "connections") void refreshRemoteAccess();
    }, remoteAccessPollIntervalMs);
    void pollIncomingRemotePairingRequests();
    remotePairingRequestPollTimer = window.setInterval(
      () => void pollIncomingRemotePairingRequests(),
      remotePairingRequestPollIntervalMs,
    );
    remoteDeviceClockTimer = window.setInterval(() => {
      remoteDevicesNowMs = Date.now();
    }, 30_000);

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
      void checkForUpdates("auto");
      eventPollTimer = window.setInterval(() => {
        void syncRecentEvents();
      }, 8000);
      updatePollTimer = window.setInterval(() => {
        void checkForUpdates("auto");
      }, updateAutoIntervalMs);
    })();

    return () => {
      disposed = true;
      runtimeObserver.dispose();
      media.removeEventListener("change", syncTheme);
      unlistenPetEvent?.();
      unlistenTokenUsage?.();
      unlistenAgentDisabled?.();
      unlistenSettings?.();
      clearEventPoll();
      clearUpdatePoll();
      clearRunningBubbleSaveTimer();
      clearRemoteAccessPoll();
      clearRemoteDeviceClock();
      clearPairingTimers();
      pairingRequestToken += 1;
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
      const [_runtimeSubscription, nextEvents, nextAppDataDir, nextPetLibrary, nextUsage, nextLaunchAtLogin] = await Promise.all([
        measureFrontendPerf("frontend.main.list_agent_runtimes", () => runtimeObserver.start()),
        measureFrontendPerf("frontend.main.recent_events", () => recentEvents()),
        measureFrontendPerf("frontend.main.app_data_directory", () => appDataDirectory()),
        measureFrontendPerf("frontend.main.list_pets", () => listPets()),
        measureFrontendPerf("frontend.main.token_usage_summary", () => tokenUsageSummary()),
        measureFrontendPerf("frontend.main.get_launch_at_login", () => getLaunchAtLoginEnabled()),
      ]);
      events = mergeEventFeed(events, nextEvents);
      appDataDir = nextAppDataDir;
      petLibrary = nextPetLibrary;
      usage = nextUsage;
      launchAtLogin = nextLaunchAtLogin;
      settings = normalizeSettings(await measureFrontendPerf("frontend.main.get_settings", () => getAppSettings()));
      void recordPerfEvent({
        name: "frontend.main.refresh",
        durationMs: performance.now() - startedAt,
        fields: {
          runtimes: agentRuntimes.length,
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

  function clearUpdatePoll() {
    if (updatePollTimer) {
      window.clearInterval(updatePollTimer);
      updatePollTimer = null;
    }
  }

  function clearRemoteAccessPoll() {
    if (remoteAccessPollTimer) {
      window.clearInterval(remoteAccessPollTimer);
      remoteAccessPollTimer = null;
    }
    if (remotePairingRequestPollTimer) {
      window.clearInterval(remotePairingRequestPollTimer);
      remotePairingRequestPollTimer = null;
    }
  }

  async function pollIncomingRemotePairingRequests() {
    if (remotePairingRequestPollBusy) return;
    remotePairingRequestPollBusy = true;
    let activeRequestId: string | null = null;
    try {
      const requests = await listRemotePairingRequests();
      const request = requests.find((candidate) =>
        !handledRemotePairingRequestIds.has(candidate.requestId)
        && candidate.expiresAt > Date.now(),
      );
      if (!request) return;
      activeRequestId = request.requestId;
      handledRemotePairingRequestIds.add(request.requestId);
      await keepWindowVisible();
      const accepted = await confirmDialog(
        `${request.descriptor.deviceName} 希望连接这台电脑。\n\n配对码：${request.confirmationCode}\n\n请确认 Remote 上显示相同配对码。`,
        { title: "确认设备配对", kind: "warning" },
      );
      await resolveRemotePairingRequest(request.requestId, accepted);
      if (accepted) await refreshRemoteAccess();
    } catch (currentError) {
      if (activeRequestId) handledRemotePairingRequestIds.delete(activeRequestId);
      remoteCommandError = remoteCommandDiagnostic(
        currentError,
        "remote_pairing_request_failed",
      );
    } finally {
      remotePairingRequestPollBusy = false;
    }
  }

  function clearRemoteDeviceClock() {
    if (remoteDeviceClockTimer) {
      window.clearInterval(remoteDeviceClockTimer);
      remoteDeviceClockTimer = null;
    }
  }

  function runtimeBusy(providerId: AgentRuntimeProviderId) {
    return busyRuntime === "all" || busyRuntime === providerId;
  }

  async function refreshRuntimes() {
    busyRuntime = "all";
    error = "";
    try {
      await runtimeObserver.refresh(refreshAgentRuntimes);
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRuntime = null;
    }
  }

  async function detectRuntime(providerId: AgentRuntimeProviderId) {
    busyRuntime = providerId;
    error = "";
    try {
      await detectAgentRuntime(providerId);
      await runtimeObserver.refresh();
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRuntime = null;
    }
  }

  async function chooseRuntimeExecutable(runtime: AgentRuntime) {
    const selected = await open({
      multiple: false,
      directory: false,
      title: `选择 ${runtime.displayName} 可执行文件`,
    });
    if (typeof selected !== "string") return;

    busyRuntime = runtime.providerId;
    error = "";
    try {
      await setAgentRuntimeExecutable(runtime.providerId, selected);
      await runtimeObserver.refresh();
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRuntime = null;
    }
  }

  async function restoreAutomaticRuntime(runtime: AgentRuntime) {
    busyRuntime = runtime.providerId;
    error = "";
    try {
      await clearAgentRuntimeExecutable(runtime.providerId);
      await runtimeObserver.refresh();
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRuntime = null;
    }
  }

  async function selectInstalledRuntime(runtime: AgentRuntime, executablePath: string) {
    busyRuntime = runtime.providerId;
    error = "";
    try {
      await setAgentRuntimeExecutable(runtime.providerId, executablePath);
      await runtimeObserver.refresh();
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRuntime = null;
    }
  }

  function runtimeIntegrationHint(runtime: AgentRuntime) {
    return `${runtime.displayName} Provider 负责探测、校验和选择自己的本机 Runtime；Host 只转发协议结果。`;
  }

  function showConnections() {
    tab = "connections";
    void refreshRemoteAccess();
  }

  async function refreshRemoteAccess() {
    if (remoteRefreshInFlight) return;
    remoteRefreshInFlight = true;
    if (!remoteRuntimeStatus && remoteDevices.length === 0) remoteClientsLoading = true;

    let statusFailed = false;
    try {
      remoteRuntimeStatus = await getRemoteAccessStatus();
      remoteCommandError = null;
    } catch (currentError) {
      statusFailed = true;
      remoteRuntimeStatus = null;
      remoteCommandError = remoteCommandDiagnostic(currentError, "remote_access_status_unavailable");
    }

    try {
      applyRemoteClientSnapshot(await listRemoteClients());
      remoteClientsUnavailable = false;
    } catch (currentError) {
      remoteClientsUnavailable = true;
      if (!remoteRuntimeStatus?.diagnostic && !statusFailed) {
        remoteCommandError = remoteCommandDiagnostic(currentError, "remote_client_list_unavailable");
      }
    } finally {
      remoteClientsLoading = false;
      remoteRefreshInFlight = false;
    }
  }

  function applyRemoteClientSnapshot(clients: Awaited<ReturnType<typeof listRemoteClients>>) {
    remoteDevices = clients.map(remoteDeviceFromClient);
    remoteDevicesNowMs = Date.now();
  }

  function remoteAccessPhaseMeta(status: RemoteAccessStatus | null, loading: boolean) {
    if (!status) {
      return {
        label: loading ? "读取中" : "不可用",
        title: loading ? "正在读取 Remote Host" : "Remote Host 未响应",
        detail: loading ? "设备管理不会阻塞其他设置。" : "其他设置仍可继续使用。",
        tone: loading ? "neutral" : "danger",
      } as const;
    }

    if (status.phase === "available") {
      return {
        label: "已就绪",
        title: status.displayName || "Remote Host 已就绪",
        detail: `${status.activeSessionCount} 个在线会话，可安全添加 Remote 客户端。`,
        tone: "ready",
      } as const;
    }
    if (status.phase === "starting") {
      return { label: "启动中", title: "Remote Host 正在启动", detail: "网络服务就绪后会自动刷新。", tone: "neutral" } as const;
    }
    if (status.phase === "unavailable") {
      return { label: "不可用", title: "Remote Host 未就绪", detail: "设备管理暂不可用，其他设置不受影响。", tone: "danger" } as const;
    }
    if (status.phase === "stopping") {
      return { label: "停止中", title: "Remote Host 正在停止", detail: "设备管理已暂停。", tone: "neutral" } as const;
    }
    return { label: "已停止", title: "Remote Host 已停止", detail: "重新启动 Code Pet 后可恢复设备管理。", tone: "neutral" } as const;
  }

  function remoteAccessCanRetry() {
    return remoteRuntimeStatus?.phase !== "stopping" && remoteRuntimeStatus?.phase !== "stopped";
  }

  async function retryRemoteAccessRuntime() {
    if (remoteRetryBusy) return;
    remoteRetryBusy = true;
    remoteCommandError = null;
    try {
      remoteRuntimeStatus = await retryRemoteAccess();
    } catch (currentError) {
      remoteCommandError = remoteCommandDiagnostic(currentError, "remote_access_retry_failed");
    } finally {
      remoteRetryBusy = false;
    }
    await refreshRemoteAccess();
  }

  async function revokeRemoteDevice(device: RemoteDevice) {
    if (device.status === "revoked" || revokingRemoteCredentialId) return;
    const confirmed = await confirmDialog(`撤销 ${device.deviceName} 的访问权限？该客户端需要重新配对才能连接。`, {
      title: "撤销 Remote 访问权限",
      kind: "warning",
    });
    if (!confirmed) return;

    revokingRemoteCredentialId = device.id;
    remoteCommandError = null;
    try {
      await revokeRemoteCredential(device.id);
      remoteDevices = remoteDevices.map((candidate) => candidate.id === device.id
        ? { ...candidate, status: "revoked" }
        : candidate);
      applyRemoteClientSnapshot(await listRemoteClients());
      remoteClientsUnavailable = false;
    } catch (currentError) {
      remoteCommandError = remoteCommandDiagnostic(currentError, "remote_credential_revoke_failed");
    } finally {
      revokingRemoteCredentialId = null;
    }
  }

  function openPairDeviceDialog() {
    pairDeviceDialogOpen = true;
    void beginRemotePairing();
  }

  async function beginRemotePairing() {
    const requestToken = ++pairingRequestToken;
    clearPairingTimers();
    activePairingId = null;
    resetPairingCopyFeedback();
    pairingKnownDeviceIds = new Set(remoteDevices.map((device) => device.id));
    pairingDisplay = {
      phase: "starting",
      qrImageUrl: null,
      expiresAtMs: null,
      remainingSeconds: null,
      pairedClientName: null,
      errorMessage: null,
    };

    try {
      const started = await startRemotePairing();
      if (requestToken !== pairingRequestToken || !pairDeviceDialogOpen) {
        void cancelRemotePairing(started.pairingId).catch(() => {});
        return;
      }

      activePairingId = started.pairingId;
      const remainingSeconds = pairingRemainingSeconds(started.expiresAt);
      pairingDisplay = {
        phase: remainingSeconds > 0 ? "waiting" : "expired",
        qrImageUrl: remainingSeconds > 0 ? started.qrSvgDataUrl : null,
        expiresAtMs: started.expiresAt,
        remainingSeconds,
        pairedClientName: null,
        errorMessage: null,
      };
      startPairingTimers(requestToken);
    } catch (currentError) {
      if (requestToken !== pairingRequestToken || !pairDeviceDialogOpen) return;
      const diagnostic = remoteCommandDiagnostic(currentError, "remote_pairing_start_failed");
      pairingDisplay = {
        phase: "error",
        qrImageUrl: null,
        expiresAtMs: null,
        remainingSeconds: null,
        pairedClientName: null,
        errorMessage: `${diagnostic.code}：${diagnostic.message}`,
      };
    }
  }

  function startPairingTimers(requestToken: number) {
    updatePairingCountdown(requestToken);
    pairingCountdownTimer = window.setInterval(() => updatePairingCountdown(requestToken), 1000);
    schedulePairingStatusPoll(requestToken);
  }

  function updatePairingCountdown(requestToken: number) {
    if (requestToken !== pairingRequestToken || !pairDeviceDialogOpen || pairingDisplay.expiresAtMs == null) return;
    const remainingSeconds = pairingRemainingSeconds(pairingDisplay.expiresAtMs);
    pairingDisplay = {
      ...pairingDisplay,
      phase: remainingSeconds > 0 && pairingDisplay.phase === "expired" ? "waiting" : remainingSeconds <= 0 ? "expired" : pairingDisplay.phase,
      qrImageUrl: remainingSeconds <= 0 ? null : pairingDisplay.qrImageUrl,
      remainingSeconds,
    };
    if (remainingSeconds <= 0 && pairingCountdownTimer) {
      resetPairingCopyFeedback();
      window.clearInterval(pairingCountdownTimer);
      pairingCountdownTimer = null;
    }
  }

  function schedulePairingStatusPoll(requestToken: number) {
    if (requestToken !== pairingRequestToken || !pairDeviceDialogOpen || !activePairingId) return;
    pairingPollTimer = window.setTimeout(() => {
      pairingPollTimer = null;
      void pollRemotePairingStatus(requestToken);
    }, 1000);
  }

  async function pollRemotePairingStatus(requestToken: number) {
    const pairingId = activePairingId;
    if (!pairingId || requestToken !== pairingRequestToken || !pairDeviceDialogOpen) return;

    try {
      const status = await getRemotePairingStatus(pairingId);
      if (requestToken !== pairingRequestToken || pairingId !== activePairingId || !pairDeviceDialogOpen) return;
      const remainingSeconds = pairingRemainingSeconds(status.expiresAt);
      const phase = pairingPhaseForStatus(status.state, remainingSeconds);

      if (phase === "success") {
        clearPairingTimers();
        activePairingId = null;
        resetPairingCopyFeedback();
        pairingDisplay = {
          ...pairingDisplay,
          phase: "success",
          qrImageUrl: null,
          remainingSeconds: 0,
          errorMessage: null,
        };
        await refreshRemoteClientsAfterPairing(requestToken);
        return;
      }

      if ((phase === "expired" && status.state === "expired") || phase === "cancelled") {
        clearPairingTimers();
        activePairingId = null;
        resetPairingCopyFeedback();
        pairingDisplay = {
          ...pairingDisplay,
          phase,
          qrImageUrl: null,
          expiresAtMs: status.expiresAt,
          remainingSeconds,
          errorMessage: null,
        };
        return;
      }

      pairingDisplay = {
        ...pairingDisplay,
        phase,
        qrImageUrl: phase === "waiting" ? (status.qrSvgDataUrl ?? null) : null,
        expiresAtMs: status.expiresAt,
        remainingSeconds,
        errorMessage: null,
      };
      schedulePairingStatusPoll(requestToken);
    } catch (currentError) {
      if (requestToken !== pairingRequestToken || pairingId !== activePairingId || !pairDeviceDialogOpen) return;
      const diagnostic = remoteCommandDiagnostic(currentError, "remote_pairing_status_failed");
      if (!diagnostic.retryable) resetPairingCopyFeedback();
      pairingDisplay = {
        ...pairingDisplay,
        phase: diagnostic.retryable ? pairingDisplay.phase : "error",
        qrImageUrl: diagnostic.retryable ? pairingDisplay.qrImageUrl : null,
        errorMessage: `${diagnostic.code}：${diagnostic.message}`,
      };
      if (diagnostic.retryable) schedulePairingStatusPoll(requestToken);
    }
  }

  async function refreshRemoteClientsAfterPairing(requestToken: number) {
    try {
      const clients = await listRemoteClients();
      if (requestToken !== pairingRequestToken || !pairDeviceDialogOpen) return;
      applyRemoteClientSnapshot(clients);
      remoteClientsUnavailable = false;
      const pairedDevice = remoteDevices.find((device) => !pairingKnownDeviceIds.has(device.id) && device.status !== "revoked");
      pairingDisplay = { ...pairingDisplay, pairedClientName: pairedDevice?.deviceName ?? null };
    } catch (currentError) {
      if (requestToken === pairingRequestToken) {
        remoteCommandError = remoteCommandDiagnostic(currentError, "remote_client_list_unavailable");
      }
    }
  }

  async function copyActivePairingJson() {
    const pairingId = activePairingId;
    const requestToken = pairingRequestToken;
    if (!pairingId || !pairingJsonCanBeCopied(pairingDisplay) || pairingCopyStatus === "copying" || pairingCopyStatus === "unavailable") return;

    pairingCopyStatus = "copying";
    pairingCopyMessage = null;
    try {
      await copyRemotePairingJson(pairingId);
      if (!pairingCopyRequestIsCurrent(requestToken, pairingId)) return;
      pairingCopyStatus = "copied";
      pairingCopyMessage = "配对 JSON 已复制，仅在当前倒计时内有效。";
    } catch (currentError) {
      if (!pairingCopyRequestIsCurrent(requestToken, pairingId)) return;
      const diagnostic = remoteCommandDiagnostic(currentError, "remote_pairing_copy_failed");
      const payloadUnavailable = diagnostic.code === "remote_pairing_payload_unavailable";
      pairingCopyStatus = payloadUnavailable ? "unavailable" : "failed";
      pairingCopyMessage = payloadUnavailable
        ? "当前配对已失效，无法复制旧的配对 JSON。"
        : "无法写入系统剪贴板，请检查权限后重试。";
    }
  }

  function pairingCopyRequestIsCurrent(requestToken: number, pairingId: string) {
    return requestToken === pairingRequestToken
      && pairingId === activePairingId
      && pairDeviceDialogOpen
      && pairingJsonCanBeCopied(pairingDisplay);
  }

  function resetPairingCopyFeedback() {
    pairingCopyStatus = "idle";
    pairingCopyMessage = null;
  }

  async function retryRemotePairing() {
    const previousPairingId = activePairingId;
    pairingRequestToken += 1;
    clearPairingTimers();
    activePairingId = null;
    resetPairingCopyFeedback();
    if (previousPairingId) {
      try {
        await cancelRemotePairing(previousPairingId);
      } catch (currentError) {
        const diagnostic = remoteCommandDiagnostic(currentError, "remote_pairing_cancel_failed");
        if (!pairingCancellationAlreadyTerminal(diagnostic)) {
          remoteCommandError = diagnostic;
        }
      }
    }
    if (pairDeviceDialogOpen) await beginRemotePairing();
  }

  async function closePairDeviceDialog() {
    const pairingId = activePairingId;
    const shouldCancel = pairingId != null && pairingDisplay.phase !== "success" && pairingDisplay.phase !== "expired" && pairingDisplay.phase !== "cancelled";
    pairingRequestToken += 1;
    clearPairingTimers();
    activePairingId = null;
    resetPairingCopyFeedback();
    pairDeviceDialogOpen = false;
    pairingDisplay = {
      phase: "unavailable",
      qrImageUrl: null,
      expiresAtMs: null,
      remainingSeconds: null,
      pairedClientName: null,
      errorMessage: null,
    };
    if (shouldCancel && pairingId) {
      try {
        await cancelRemotePairing(pairingId);
      } catch (currentError) {
        const diagnostic = remoteCommandDiagnostic(currentError, "remote_pairing_cancel_failed");
        if (!pairingCancellationAlreadyTerminal(diagnostic)) {
          remoteCommandError = diagnostic;
        }
      }
    }
    await tick();
    addDeviceButton?.focus();
  }

  function pairingCancellationAlreadyTerminal(diagnostic: RemoteAccessDiagnostic) {
    return diagnostic.code === "pairing_session_not_active" || diagnostic.code === "pairing_session_not_found";
  }

  function clearPairingTimers() {
    if (pairingPollTimer) {
      window.clearTimeout(pairingPollTimer);
      pairingPollTimer = null;
    }
    if (pairingCountdownTimer) {
      window.clearInterval(pairingCountdownTimer);
      pairingCountdownTimer = null;
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

  async function checkForUpdates(mode: UpdateCheckMode) {
    if (updateCheckMode || updateInstallBusy) {
      return;
    }

    updateCheckMode = mode;
    updateError = "";
    if (mode === "manual") {
      updateMessage = "Checking...";
    }

    try {
      const update = await checkAppUpdate();
      if (!shouldPromptForUpdate(update, mode, settings)) {
        if (mode === "manual") {
          updateMessage = "Latest version installed.";
        }
        return;
      }

      availableUpdate = update;
      updatePromptMode = mode;
      updateMessage = `Version ${update.version} available.`;
    } catch (currentError) {
      if (mode === "manual") {
        updateError = String(currentError);
        updateMessage = "";
      }
    } finally {
      updateCheckMode = null;
    }
  }

  async function dismissAvailableUpdate() {
    const update = availableUpdate;
    availableUpdate = null;
    if (!update || !settings) {
      return;
    }

    const nextSettings = normalizeSettings({
      ...settings,
      updates: ignoredUpdateSettings(update.version),
    });
    settings = nextSettings;
    updateMessage = updatePromptMode === "manual" ? `Version ${update.version} skipped.` : "";
    updateError = "";
    try {
      settings = normalizeSettings(await updateAppSettings(nextSettings));
    } catch (currentError) {
      updateError = String(currentError);
    }
  }

  async function installAvailableUpdate() {
    if (!availableUpdate || updateInstallBusy) {
      return;
    }

    updateInstallBusy = true;
    updateError = "";
    updateMessage = `Installing ${availableUpdate.version}...`;
    try {
      await installAppUpdate();
    } catch (currentError) {
      updateInstallBusy = false;
      updateError = String(currentError);
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
      const filename = await basename(selected, `.${await extname(selected)}`) || "Imported Pet";
      const sourcePath = cutOutSubject ? (await cutOutImageSubject(selected, await cutoutOutputPath(selected))).outputPath : selected;
      petLibrary = await importPetImage(sourcePath, filename, settings?.pet.imagePixelSize ?? defaultImagePixelSize);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function cutoutOutputPath(sourcePath: string) {
    const baseDirectory = petLibrary?.dataDirectory ?? settings?.petLibrary.dataDirectory ?? "";
    const filename = await basename(sourcePath, `.${await extname(sourcePath)}`) || "subject";
    const timestamp = Date.now();
    return baseDirectory
      ? await join(baseDirectory, "cutouts", `${filename}-${timestamp}.png`)
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
    busyAppDataDirectory = true;
    error = "";
    let clearTarget = false;
    try {
      const targetStatus = await appDataDirectoryTargetStatus(selected);
      if (targetStatus.requiresClear) {
        const confirmed = await confirmDialog("所选目录已有内容。继续会先清空该目录，再复制当前 Code Pet 数据。是否继续？", {
          title: "清空数据目录",
          kind: "warning",
          okLabel: "清空并使用",
          cancelLabel: "取消",
        });
        if (!confirmed) return;
        clearTarget = true;
      }
    } catch (currentError) {
      error = String(currentError);
      return;
    } finally {
      busyAppDataDirectory = false;
    }
    await updateAppDataDirectory(selected, clearTarget);
  }

  async function resetAppDataDirectory() {
    await updateAppDataDirectory(null);
  }

  async function updateAppDataDirectory(path: string | null, clearTarget = false) {
    busyAppDataDirectory = true;
    error = "";
    try {
      settings = normalizeSettings(await setAppDataDirectory(path, clearTarget));
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
    nextSettings.notifications.robot = normalizeRobotNotificationSettings(nextSettings.notifications.robot);
    nextSettings.activityFilters = normalizeActivityFilters(nextSettings.activityFilters);
    nextSettings.agents = normalizeAgentSettings(nextSettings.agents);
    nextSettings.agentRuntimes = {
      byProvider: {
        ...agentRuntimeSettingsDefaults.byProvider,
        ...(nextSettings.agentRuntimes?.byProvider ?? {}),
      },
    };
    nextSettings.providerPlugins = {
      directories: [...nextSettings.providerPlugins.directories],
    };
    nextSettings.updates = {
      ignoredVersion: nextSettings.updates?.ignoredVersion ?? null,
    };
    return nextSettings;
  }

  function normalizeRobotNotificationSettings(robot: Partial<AppSettings["notifications"]["robot"]> | null | undefined): AppSettings["notifications"]["robot"] {
    return {
      enabled: Boolean(robot?.enabled),
      triggers: {
        ...robotNotificationDefaults.triggers,
        ...(robot?.triggers ?? {}),
      },
      template: normalizeRobotTemplate(robot?.template),
      channels: (robot?.channels ?? []).map(normalizeRobotChannel).filter((channel): channel is RobotNotificationChannel => Boolean(channel)),
    };
  }

  function normalizeRobotTemplate(template: Partial<AppSettings["notifications"]["robot"]["template"]> | null | undefined): AppSettings["notifications"]["robot"]["template"] {
    return {
      ...robotNotificationDefaults.template,
      ...(template ?? {}),
    };
  }

  function normalizeRobotChannel(channel: Partial<RobotNotificationChannel> | null | undefined): RobotNotificationChannel | null {
    if (!channel) return null;
    if (channel.provider === "feishu") {
      const feishu = channel;
      return {
        provider: "feishu",
        id: feishu.id || robotChannelId("feishu"),
        name: feishu.name || "飞书机器人",
        enabled: feishu.enabled ?? true,
        webhookUrl: feishu.webhookUrl ?? "",
        webhookSecret: feishu.webhookSecret ?? "",
      };
    }
    if (channel.provider === "dingtalk") {
      const dingtalk = channel;
      return {
        provider: "dingtalk",
        id: dingtalk.id || robotChannelId("dingtalk"),
        name: dingtalk.name || "钉钉机器人",
        enabled: dingtalk.enabled ?? true,
        authMode: dingtalk.authMode ?? "enterprise-robot",
        targetType: dingtalk.targetType ?? "user-ids",
        robotCode: dingtalk.robotCode ?? "",
        clientId: dingtalk.clientId ?? "",
        clientSecret: dingtalk.clientSecret ?? "",
        userIds: normalizeRobotList(dingtalk.userIds),
        openConversationId: dingtalk.openConversationId ?? "",
        webhookUrl: dingtalk.webhookUrl ?? "",
        webhookSecret: dingtalk.webhookSecret ?? "",
      };
    }
    return null;
  }

  function robotChannelId(provider: RobotNotificationChannel["provider"]) {
    return `${provider}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  }

  function createRobotChannel(provider: RobotNotificationChannel["provider"]): RobotNotificationChannel {
    if (provider === "feishu") {
      return {
        provider,
        id: robotChannelId(provider),
        name: "飞书机器人",
        enabled: true,
        webhookUrl: "",
        webhookSecret: "",
      };
    }
    return {
      provider,
      id: robotChannelId(provider),
      name: "钉钉机器人",
      enabled: true,
      authMode: "enterprise-robot",
      targetType: "user-ids",
      robotCode: "",
      clientId: "",
      clientSecret: "",
      userIds: [],
      openConversationId: "",
      webhookUrl: "",
      webhookSecret: "",
    };
  }

  async function addRobotChannel(provider: RobotNotificationChannel["provider"]) {
    if (!settings) return;
    settings.notifications.robot = normalizeRobotNotificationSettings({
      ...settings.notifications.robot,
      enabled: true,
      channels: [...settings.notifications.robot.channels, createRobotChannel(provider)],
    });
    robotNotificationResult = "";
    await saveSettings();
  }

  async function removeRobotChannel(channelId: string) {
    if (!settings) return;
    settings.notifications.robot.channels = settings.notifications.robot.channels.filter((channel) => channel.id !== channelId);
    robotNotificationResult = "";
    await saveSettings();
  }

  async function testRobotChannel(channelId: string) {
    if (!settings) return;
    busyRobotChannel = channelId;
    robotNotificationResult = "";
    error = "";
    try {
      await saveSettings();
      robotNotificationResult = await sendTestRobotNotification(channelId);
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyRobotChannel = "";
    }
  }

  function robotChannelLabel(channel: RobotNotificationChannel) {
    if (channel.provider === "feishu") {
      return channel.name || "飞书机器人";
    }
    return channel.name || (channel.authMode === "webhook" ? "钉钉 webhook" : "钉钉企业机器人");
  }

  function robotChannelMeta(channel: RobotNotificationChannel) {
    if (channel.provider === "feishu") {
      return "飞书 webhook";
    }
    return channel.authMode === "webhook"
      ? "钉钉 webhook"
      : channel.targetType === "open-conversation-id"
        ? "钉钉企业机器人 · 群"
        : "钉钉企业机器人 · 用户";
  }

  function dingTalkUserIdsValue(channel: DingTalkRobotChannel) {
    return normalizeRobotList(channel.userIds).join(", ");
  }

  function updateDingTalkUserIds(channel: DingTalkRobotChannel, value: string) {
    channel.userIds = normalizeRobotList(value.split(/[,\n，\s]+/));
  }

  async function updateRobotTemplateField(key: keyof AppSettings["notifications"]["robot"]["template"], value: string) {
    if (!settings) return;
    settings.notifications.robot.template = normalizeRobotTemplate({
      ...settings.notifications.robot.template,
      [key]: value,
    });
    await saveSettings();
  }

  async function resetRobotTemplate() {
    if (!settings) return;
    settings.notifications.robot.template = normalizeRobotTemplate(null);
    await saveSettings();
  }

  function normalizeRobotList(values: Array<string | null | undefined> | null | undefined): string[] {
    const seen = new Set<string>();
    const normalized: string[] = [];
    for (const rawValue of values ?? []) {
      const value = (rawValue ?? "").trim();
      if (!value || seen.has(value)) {
        continue;
      }
      seen.add(value);
      normalized.push(value);
    }
    return normalized;
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
    return { byAgent: {} };
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
      "waiting-approval": "等待审批或输入",
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
  $: enabledSources = petSources.filter(source => source.enabled);
  $: receivingSources = petSources.filter(source => source.status === "receiving");
  $: usageData = buildUsageChartData(usage, { range: usageRange, bucketSize: usageBucketSize });
  $: usageBuckets = usageData.buckets;
  $: usageMaxTokens = usageData.maxTokens;
  $: usageTickLabels = yAxisTicks(usageMaxTokens);
  $: pageTitle = tab === "agents" ? "Agent" : tab === "connections" ? "连接" : tab === "usage" ? "用量" : tab === "personalize" ? "个性化" : "最新事件";
  $: appTheme = themeClassNames(settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "dark" : "light");
</script>

<main class={`app-shell main-theme ${appTheme}`} class:sidebar-collapsed={sidebarCollapsed}>
  <WindowToolbar bind:collapsed={sidebarCollapsed} onError={(message) => error = message} />
  <aside id="main-sidebar" class="sidebar" inert={sidebarCollapsed}>
    <div class="brand"><h1>Code Pet</h1></div>
    <nav class="tabs" aria-label="Code Pet settings">
      <button class:active={tab === "agents"} on:click={() => (tab = "agents")} aria-label="Agent 列表">
        <Bot size={18} /> Agent
      </button>
      <button class:active={tab === "connections"} on:click={showConnections} aria-label="设备与本机运行时连接">
        <Cable size={18} /> 连接
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

  <section class="content-pane">
    <header class="topbar">
      <div>
        <h2>{pageTitle}</h2>
        {#if error}<p class="error">{error}</p>{/if}
      </div>
    </header>

    <div class="content">
    {#if tab === "agents"}
      <div class="agent-workspace">
        <section class="overview-grid" aria-label="运行概览">
          <article class="overview-card pixel-panel">
            <span><PlugZap size={17} /> 活动来源</span>
            <strong>{enabledSources.length}/{petSources.length}</strong>
            <p>已启用的任务活动来源</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><Activity size={17} /> 接收状态</span>
            <strong>{receivingSources.length}</strong>
            <p>已收到活动的来源</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><ShieldAlert size={17} /> 桌宠任务列表</span>
            <strong>只读</strong>
            <p>回复与授权请在原应用处理</p>
          </article>
        </section>

        <section class="agent-section pixel-panel">
          <header class="section-head">
            <div>
              <span class="agent-kicker">CONNECTED AGENTS</span>
              <h3>接入状态</h3>
            </div>
            <span>Codex · Claude · OpenCode</span>
          </header>

          <PetSources bind:sources={petSources} />
        </section>
      </div>
    {:else if tab === "connections"}
      <div class="connection-workspace">
        <section class="device-section pixel-panel">
          <header class="section-head connection-section-head">
            <div>
              <span class="agent-kicker">PAIRED DEVICES</span>
              <h3>设备</h3>
              <p>管理已配对的 Remote 客户端及其访问权限。</p>
            </div>
            <button bind:this={addDeviceButton} class="connection-primary-button" type="button" disabled={remoteRuntimeStatus?.phase !== "available" || pairDeviceDialogOpen} on:click={openPairDeviceDialog}>
              <Plus size={17} /> 添加设备
            </button>
          </header>

          <div class="remote-access-summary" role="status">
            <span class="remote-access-summary-icon" aria-hidden="true"><PlugZap size={19} /></span>
            <div>
              <strong>{remoteAccessPhaseMeta(remoteRuntimeStatus, remoteClientsLoading).title}</strong>
              <span>{remoteAccessPhaseMeta(remoteRuntimeStatus, remoteClientsLoading).detail}</span>
              {#if remoteRuntimeStatus?.networkInterface}
                <span class="remote-network-interface">网卡：{remoteRuntimeStatus.networkInterface.name} · {remoteRuntimeStatus.networkInterface.kind === "wifi" ? "Wi-Fi" : remoteRuntimeStatus.networkInterface.kind === "ethernet" ? "以太网" : "网络接口"} · {remoteRuntimeStatus.networkInterface.ipv4}</span>
              {/if}
            </div>
            <span
              class:online={remoteAccessPhaseMeta(remoteRuntimeStatus, remoteClientsLoading).tone === "ready"}
              class:runtime-danger={remoteAccessPhaseMeta(remoteRuntimeStatus, remoteClientsLoading).tone === "danger"}
              class="status-chip"
            >
              {remoteAccessPhaseMeta(remoteRuntimeStatus, remoteClientsLoading).label}
            </span>
          </div>

          {#if visibleRemoteDiagnostic}
            <div class="runtime-diagnostic remote-access-diagnostic" role="alert">
              <ShieldAlert size={17} />
              <div>
                <strong>{visibleRemoteDiagnostic.code}</strong>
                <p>{visibleRemoteDiagnostic.message}</p>
              </div>
              {#if remoteAccessCanRetry()}
                <button type="button" disabled={remoteRetryBusy} on:click={retryRemoteAccessRuntime}>
                  <RefreshCw size={16} /> {remoteRetryBusy ? "重试中" : "重试"}
                </button>
              {/if}
            </div>
          {/if}

          <RemoteDeviceList
            devices={remoteDevices}
            nowMs={remoteDevicesNowMs}
            loading={remoteClientsLoading}
            unavailable={remoteClientsUnavailable}
            revokingDeviceId={revokingRemoteCredentialId}
            onRevoke={revokeRemoteDevice}
          />
        </section>

        <section class="runtime-section pixel-panel">
          <header class="section-head runtime-section-head">
            <div>
              <span class="agent-kicker">LOCAL EXECUTABLES</span>
              <h3>本机运行时</h3>
              <p>Code Pet 会验证可执行文件和版本；手动配置始终优先于自动检测。</p>
            </div>
            <button class="runtime-refresh-button" type="button" disabled={busyRuntime !== null} on:click={refreshRuntimes}>
              <RefreshCw size={17} /> {busyRuntime === "all" ? "检测中" : "重新检测"}
            </button>
          </header>

          <div class="runtime-list">
            {#each agentRuntimes as runtime}
              {@const status = agentRuntimeStatusMeta(runtime)}
              <article class="runtime-card">
                <header class="runtime-card-head">
                  <div>
                    <span class="agent-kicker">{runtime.providerId}</span>
                    <h3>{runtime.displayName}</h3>
                    <p>{runtimeIntegrationHint(runtime)}</p>
                  </div>
                  <span class:online={status.tone === "ready"} class:runtime-danger={status.tone === "danger"} class="status-chip">{status.label}</span>
                </header>

                <ProviderConnectionStatus current={providerConnections.find((state) => state.providerId === runtime.providerId)} available={providerConnectionsLoaded} />
                <dl class="runtime-meta">
                  <div>
                    <dt>当前路径</dt>
                    <dd><code title={runtime.resolvedExecutable ?? ""}>{runtime.resolvedExecutable ?? "—"}</code></dd>
                  </div>
                  <div>
                    <dt>来源</dt>
                    <dd>{agentRuntimeSourceLabel(runtime.source)}</dd>
                  </div>
                  <div>
                    <dt>版本</dt>
                    <dd>{runtime.version ?? "—"}</dd>
                  </div>
                  <div>
                    <dt>用户选择</dt>
                    <dd><code title={runtime.configuredExecutable ?? ""}>{runtime.configuredExecutable ?? "自动选择"}</code></dd>
                  </div>
                </dl>

                <div class="runtime-installations">
                  <strong>Provider 探测到的安装</strong>
                  {#each runtime.installed ?? [] as installation}
                    <button
                      type="button"
                      class:selected={runtime.resolvedExecutable === installation.executablePath}
                      disabled={runtimeBusy(runtime.providerId) || runtime.status === "loading"}
                      on:click={() => selectInstalledRuntime(runtime, installation.executablePath)}
                    >
                      <span><b>{installation.version}</b> · {agentRuntimeSourceLabel(installation.source)}</span>
                      <code title={installation.executablePath}>{installation.executablePath}</code>
                    </button>
                  {:else}
                    <p>{runtime.status === "loading" ? "Provider 启动后将自动检测本机安装。" : "没有通过 Provider 校验的本机安装。"}</p>
                  {/each}
                </div>

                {#if runtime.diagnostic}
                  <div class="runtime-diagnostic" role="status">
                    <ShieldAlert size={17} />
                    <div>
                      <strong>{runtime.diagnostic.code}</strong>
                      <p>{runtime.diagnostic.message}</p>
                    </div>
                  </div>
                {/if}

                <div class="runtime-actions">
                  <button type="button" disabled={runtimeBusy(runtime.providerId) || runtime.status === "loading"} on:click={() => detectRuntime(runtime.providerId)}>
                    <RefreshCw size={16} /> 检测
                  </button>
                  <button type="button" disabled={runtimeBusy(runtime.providerId) || runtime.status === "loading"} on:click={() => chooseRuntimeExecutable(runtime)}>
                    <FolderOpen size={16} /> 选择路径
                  </button>
                  <button type="button" disabled={runtimeBusy(runtime.providerId) || runtime.status === "loading" || !canRestoreAutomaticDetection(runtime)} on:click={() => restoreAutomaticRuntime(runtime)}>
                    <RotateCcw size={16} /> 恢复自动检测
                  </button>
                </div>
              </article>
            {:else}
              <div class="empty-state">
                <Cpu size={20} />
                <strong>运行时状态尚未加载</strong>
                <p>点击重新检测以读取本机 Agent 安装状态。</p>
              </div>
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
                {appDataRestartPending ? "已保存并复制原数据，重启后完全生效。" : "所选目录需为空；若非空会先确认清空，再复制原数据，保存后请重启。"}
              </p>
            </div>
            <label class="check">
              <input type="checkbox" checked={launchAtLogin} disabled={busyLaunchAtLogin} on:change={toggleLaunchAtLogin} />
              开机自启动
            </label>
            <div class="update-settings">
              <div class="setting-line">
                <span>App updates</span>
                <em>{settings.updates.ignoredVersion ? `Ignored ${settings.updates.ignoredVersion}` : "Ready"}</em>
              </div>
              <div class="update-status-row">
                <span>{updateError || updateMessage || "Ready"}</span>
                <button disabled={!!updateCheckMode || updateInstallBusy} on:click={() => checkForUpdates("manual")}>
                  <RefreshCw size={16} /> {updateCheckMode === "manual" ? "Checking" : "Check"}
                </button>
              </div>
            </div>
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

          <section class="robot-editor pixel-panel">
            <header class="panel-head">
              <div>
                <h3><Bot size={18} /> 通知机器人</h3>
              </div>
              <label class="switch-check">
                <input type="checkbox" bind:checked={settings.notifications.robot.enabled} on:change={saveSettings} />
                启用
              </label>
            </header>

            <div class="robot-trigger-grid" aria-label="机器人触发事件">
              {#each robotTriggerOptions as trigger}
                <label class="check">
                  <input type="checkbox" bind:checked={settings.notifications.robot.triggers[trigger.key]} on:change={saveSettings} />
                  {trigger.label}
                </label>
              {/each}
            </div>

            <section class="robot-template-panel">
              <div class="robot-template-head">
                <strong>消息模板</strong>
                <button type="button" on:click={resetRobotTemplate}>恢复默认</button>
              </div>
              <code>{robotTemplateVariables}</code>
              <div class="robot-template-grid">
                {#each robotTemplateFields as field}
                  <label>
                    {field.label}
                    <textarea
                      rows={field.rows}
                      value={settings.notifications.robot.template[field.key]}
                      on:change={(event) => updateRobotTemplateField(field.key, inputValue(event))}
                    ></textarea>
                  </label>
                {/each}
              </div>
            </section>

            <div class="row-actions">
              <button on:click={() => addRobotChannel("dingtalk")}>
                <Bot size={17} /> 钉钉
              </button>
              <button on:click={() => addRobotChannel("feishu")}>
                <Bot size={17} /> 飞书
              </button>
            </div>

            {#if robotNotificationResult}
              <p class="setting-note robot-result">{robotNotificationResult}</p>
            {/if}

            <div class="robot-channel-list" aria-label="机器人通知渠道">
              {#if settings.notifications.robot.channels.length}
                {#each settings.notifications.robot.channels as channel (channel.id)}
                  <article class="robot-channel-card">
                    <div class="robot-channel-head">
                      <span class={`robot-provider ${channel.provider}`}>{channel.provider === "dingtalk" ? "钉" : "飞"}</span>
                      <div>
                        <strong>{robotChannelLabel(channel)}</strong>
                        <em>{robotChannelMeta(channel)}</em>
                      </div>
                      <label class="switch-check compact">
                        <input type="checkbox" bind:checked={channel.enabled} on:change={saveSettings} />
                        启用
                      </label>
                    </div>

                    <label>
                      名称
                      <input type="text" bind:value={channel.name} on:change={saveSettings} />
                    </label>

                    {#if channel.provider === "dingtalk"}
                      <div class="segmented">
                        <button
                          class:active={channel.authMode === "enterprise-robot"}
                          on:click={async () => {
                            channel.authMode = "enterprise-robot";
                            await saveSettings();
                          }}
                        >
                          企业机器人
                        </button>
                        <button
                          class:active={channel.authMode === "webhook"}
                          on:click={async () => {
                            channel.authMode = "webhook";
                            await saveSettings();
                          }}
                        >
                          Webhook
                        </button>
                      </div>

                      {#if channel.authMode === "enterprise-robot"}
                        <div class="robot-field-grid">
                          <label>
                            robotCode
                            <input type="text" bind:value={channel.robotCode} on:change={saveSettings} />
                          </label>
                          <label>
                            clientId
                            <input type="text" bind:value={channel.clientId} on:change={saveSettings} />
                          </label>
                        </div>
                        <label>
                          clientSecret
                          <input type="password" bind:value={channel.clientSecret} on:change={saveSettings} />
                        </label>
                        <div class="segmented">
                          <button
                            class:active={channel.targetType === "user-ids"}
                            on:click={async () => {
                              channel.targetType = "user-ids";
                              await saveSettings();
                            }}
                          >
                            用户
                          </button>
                          <button
                            class:active={channel.targetType === "open-conversation-id"}
                            on:click={async () => {
                              channel.targetType = "open-conversation-id";
                              await saveSettings();
                            }}
                          >
                            群
                          </button>
                        </div>
                        {#if channel.targetType === "user-ids"}
                          <label>
                            userId
                            <textarea rows="2" value={dingTalkUserIdsValue(channel)} on:change={(event) => {
                              updateDingTalkUserIds(channel, inputValue(event));
                              void saveSettings();
                            }}></textarea>
                          </label>
                        {:else}
                          <label>
                            openConversationId
                            <input type="text" bind:value={channel.openConversationId} on:change={saveSettings} />
                          </label>
                        {/if}
                      {:else}
                        <label>
                          webhook
                          <input type="url" bind:value={channel.webhookUrl} on:change={saveSettings} />
                        </label>
                        <label>
                          加签密钥
                          <input type="password" bind:value={channel.webhookSecret} on:change={saveSettings} />
                        </label>
                      {/if}
                    {:else}
                      <label>
                        webhook
                        <input type="url" bind:value={channel.webhookUrl} on:change={saveSettings} />
                      </label>
                      <label>
                        签名密钥
                        <input type="password" bind:value={channel.webhookSecret} on:change={saveSettings} />
                      </label>
                    {/if}

                    <div class="robot-channel-actions">
                      <button disabled={busyRobotChannel === channel.id} on:click={() => testRobotChannel(channel.id)}>
                        <PlugZap size={17} /> 测试
                      </button>
                      <button class="danger-action" on:click={() => removeRobotChannel(channel.id)} aria-label={`删除 ${robotChannelLabel(channel)}`}>
                        <Trash2 size={17} /> 删除
                      </button>
                    </div>
                  </article>
                {/each}
              {:else}
                <div class="empty-state compact">
                  <Bot size={20} />
                  <strong>未配置机器人</strong>
                  <p>添加一个钉钉或飞书渠道后，任务状态会按上面的事件发送。</p>
                </div>
              {/if}
            </div>
          </section>
        </div>
      </div>
    {:else if tab === "events"}
      <EventJournal />
    {/if}
    </div>
  </section>

  <PairDeviceDialog
    open={pairDeviceDialogOpen}
    display={pairingDisplay}
    canCopyPairingJson={activePairingId != null && pairingJsonCanBeCopied(pairingDisplay)}
    copyStatus={pairingCopyStatus}
    copyMessage={pairingCopyMessage}
    onCopyPairingJson={copyActivePairingJson}
    onClose={closePairDeviceDialog}
    onRetry={retryRemotePairing}
  />
{#if availableUpdate}
  <div class="modal-scrim">
    <div class="update-dialog pixel-panel" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title">
      <header class="panel-head">
        <div>
          <h3 id="update-dialog-title"><Download size={18} /> Update available</h3>
        </div>
        <button class="icon-button" type="button" disabled={updateInstallBusy} on:click={dismissAvailableUpdate} aria-label="Cancel update">
          <X size={17} />
        </button>
      </header>
      <p>Code Pet {availableUpdate.version} is ready. Current version: {availableUpdate.currentVersion}.</p>
      {#if updateError}
        <p class="error">{updateError}</p>
      {/if}
      <div class="row-actions">
        <button type="button" disabled={updateInstallBusy} on:click={dismissAvailableUpdate}>Cancel</button>
        <button class="primary-action" type="button" disabled={updateInstallBusy} on:click={installAvailableUpdate}>
          <Download size={17} /> {updateInstallBusy ? "Installing" : "Upgrade now"}
        </button>
      </div>
    </div>
  </div>
{/if}
</main>
