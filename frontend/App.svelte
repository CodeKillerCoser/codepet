<script lang="ts">
  import { Button } from "./lib/ui/button/index";
  import { Switch } from "./lib/ui/switch/index";
  import UpdateDialog from "./lib/settings/UpdateDialog.svelte";
  import HorizontalTabs from "./lib/HorizontalTabs.svelte";
  import SettingsRow from "./lib/SettingsRow.svelte";
  import AboutSettings from "./lib/AboutSettings.svelte";
  import PermissionSettings from "./lib/PermissionSettings.svelte";
  import { consumeReloadNotice, webcontentReloading } from "./lib/webcontent";
  import EventJournal from "./lib/EventJournal.svelte";
  import AppSidePanelHost from "./lib/shell/AppSidePanelHost.svelte";
  import { provideSidePanel } from "./lib/shell/sidePanel";
  import CodexWorkbench from "./lib/CodexWorkbench.svelte";
  import MainNavigation from "./lib/MainNavigation.svelte";
  import { createNavigation, connectionRoutes, isConnectionRoute, isSettingsRoute, routeTitle, type AppRoute } from "./lib/navigation";
  import WindowToolbar from "./lib/WindowToolbar.svelte";
  import { basename, extname, join } from "@tauri-apps/api/path";
  import PetSources from "./lib/PetSources.svelte";
  import type { NoticeSource } from "./lib/noticeGateway";
  import { confirm as confirmDialog, open } from "@tauri-apps/plugin-dialog";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Activity, FolderCog, PlugZap, Plus, RefreshCw, ShieldAlert } from "@lucide/svelte";
  import { onMount, tick } from "svelte";
  import ProviderRuntimeInfo from "./lib/ProviderRuntimeInfo.svelte";
  import { shouldShowRuntime, isBuiltinProvider } from "./lib/agentRuntime";
  import { observeProviderRuntimes, type ProviderConnectionState } from "./lib/providerRuntimes";
  import { getAppPaths, removeProviderPlugin, importProviderPlugin, type AppPaths, checkAppUpdate, cutOutImageSubject, deletePet, getAppSettings, getLaunchAtLoginEnabled, importPetImage, installAppUpdate, listPets, recentEvents, recordPerfEvent, refreshAgentRuntimes, selectPet, setAgentRuntimeExecutable, setPathManagerData, setLaunchAtLoginEnabled, setPathManagerPets, updateAppSettings, importPetPackage } from "./lib/api";
  import { mergeEventFeed } from "./lib/eventFeed";
  import PetLibraryPanel from "./lib/pet/PetLibraryPanel.svelte";
  import PairDeviceDialog from "./lib/PairDeviceDialog.svelte";
  import RemoteDeviceList from "./lib/RemoteDeviceList.svelte";
  import { cancelRemotePairing, copyRemotePairingJson, getRemoteAccessStatus, getRemotePairingStatus, listRemoteClients, listRemotePairingRequests, remoteCommandDiagnostic, resolveRemotePairingRequest, retryRemoteAccess, revokeRemoteCredential, deleteRemoteClient, startRemotePairing, type RemoteAccessDiagnostic, type RemoteAccessStatus } from "./lib/remoteAccess";
  import { pairingJsonCanBeCopied, pairingPhaseForStatus, pairingRemainingSeconds, remoteDeviceFromClient, type PairingCopyStatus, type PairingDisplayState, type RemoteDevice } from "./lib/remoteDevices";
  import { themeClassNames } from "./lib/theme";
  import { ignoredUpdateSettings, shouldPromptForUpdate, type UpdateCheckMode } from "./lib/updates";
  import UsagePanel from "./lib/UsagePanel.svelte";
  import type { AgentRuntime, AppSettings, AppUpdate, PetEvent, PetLibraryView } from "./lib/types";
  import SignalingServiceSettings from "./lib/SignalingServiceSettings.svelte";
  import { configureSignalingService } from "./lib/api";

  import AppearanceSettings from "./lib/settings/AppearanceSettings.svelte";
  import NotificationSettings from "./lib/settings/NotificationSettings.svelte";
  import { normalizeSettings } from "./lib/settings/model";

  const sidePanel = provideSidePanel();
  const sidePanelCurrent = sidePanel.current;
  const sidePanelMode = sidePanel.mode;
  const resourceReloaded = consumeReloadNotice();
  const navigation = createNavigation(resourceReloaded ? "about" : "tasks");
  let pendingSettingsSaves = 0;
  $: tab = $navigation.current;
  $: if (tab === "devices") void refreshRemoteAccess();
  let sidebarCollapsed = false;
  let contentElement: HTMLDivElement;
  let pageHeading: HTMLHeadingElement;
  let renderedRoute: AppRoute = "tasks";
  $: if (tab !== renderedRoute) {
    sidePanel.dismiss(false);
    renderedRoute = tab;
    void tick().then(() => {
      contentElement?.scrollTo({ top: 0 });
      if (!document.activeElement?.closest('[role="tablist"]')) {
        const heading = tab === "tasks" || tab === "extraction" ? contentElement?.querySelector<HTMLElement>(".workbench") : pageHeading;
        heading?.focus({ preventScroll: true });
      }
    });
  }
  let petSources: NoticeSource[] = [];
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
  let events: PetEvent[] = [];
  let appDataDir = "";
  let managedPaths: AppPaths | null = null;
  let busyRuntime: string | null = null;
  let busyPet = "";
  let busyAppDataDirectory = false;
  let appDataRestartPending = false;
  let busyLaunchAtLogin = false;
  let busyRobotChannel = "";
  let error = "";
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
  let appearancePending = false;
  let appearanceVisited = false;
  $: if (tab === "appearance") appearanceVisited = true;
  const updateAutoIntervalMs = 6 * 60 * 60 * 1000;
  const remoteAccessPollIntervalMs = 1_000;
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
      if (tab === "devices") void refreshRemoteAccess();
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
    let unlistenAgentDisabled: (() => void) | null = null;
    let unlistenSettings: (() => void) | null = null;
    void (async () => {
      await keepWindowVisible();
      unlistenPetEvent = await listen<PetEvent>("pet-event", (event) => {
        events = mergeEventFeed(events, [event.payload]);
      });
      unlistenAgentDisabled = await listen<string>("agent-disabled", (event) => {
        events = events.filter((activity) => activity.provider !== event.payload);
      });
      unlistenSettings = await listen<AppSettings>("settings-updated", (event) => {
        settings = normalizeSettings(event.payload);
      });
      if (disposed) {
        unlistenPetEvent();
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
      unlistenAgentDisabled?.();
      unlistenSettings?.();
      clearEventPoll();
      clearUpdatePoll();
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
      const [_runtimeSubscription, nextEvents, nextAppDataDir, nextPetLibrary, nextLaunchAtLogin] = await Promise.all([
        measureFrontendPerf("frontend.main.list_agent_runtimes", () => runtimeObserver.start()),
        measureFrontendPerf("frontend.main.recent_events", () => recentEvents()),
        measureFrontendPerf("frontend.main.app_data_directory", () => getAppPaths()),
        measureFrontendPerf("frontend.main.list_pets", () => listPets()),
        measureFrontendPerf("frontend.main.get_launch_at_login", () => getLaunchAtLoginEnabled()),
      ]);
      events = mergeEventFeed(events, nextEvents);
      managedPaths = nextAppDataDir;
      appDataDir = nextAppDataDir.data;
      petLibrary = nextPetLibrary;
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

  let providerAddButton: HTMLButtonElement;
  let providerImportNotice = "";
  let providerOperationError = "";
  let providerErrorElement: HTMLParagraphElement;
  let providerImportBusy = false;

  async function removeCustomProvider(runtime: AgentRuntime) {
    if (providerImportBusy || busyRuntime !== null) return;
    if (!await confirmDialog(`移除 ${runtime.displayName}？这会停止该 Provider 的运行任务，并从加载列表移除。安装的插件包会删除，相关进程和连接会关闭，历史和数据库保留。`, { title: "移除自定义 Provider", kind: "warning", okLabel: "移除", cancelLabel: "取消" })) return;
    providerImportBusy = true;
    busyRuntime = runtime.providerId;
    providerImportNotice = "";
    providerOperationError = "";
    error = "";
    try {
      await removeProviderPlugin(runtime.providerId);
      agentRuntimes = agentRuntimes.filter(item => item.providerId !== runtime.providerId);
      providerConnections = providerConnections.filter(item => item.providerId !== runtime.providerId);
      providerImportNotice = `${runtime.displayName} 已卸载，安装包已删除，历史数据已保留。`;
      try { await runtimeObserver.refresh(); }
      catch { providerOperationError = "已移除，但刷新列表失败，请重新打开本机运行时页面。"; }
    } catch (currentError) { providerOperationError = `移除 ${runtime.displayName} 失败：${String(currentError)}`; }
    finally { providerImportBusy = false; busyRuntime = null; await tick(); if (providerOperationError) providerErrorElement?.focus(); else providerAddButton?.focus(); }
  }

  async function addCustomProvider() {
    if (providerImportBusy) return;
    providerImportBusy = true;
    providerImportNotice = "";
    providerOperationError = "";
    error = "";
    try {
      const manifest = await open({ multiple: false, directory: false,
        title: "选择 codepet-provider.json", filters: [{ name: "Provider manifest", extensions: ["json"] }] });
      if (typeof manifest !== "string") return;
      if (await basename(manifest) !== "codepet-provider.json") {
        throw new Error("请选择 Provider 的 codepet-provider.json 文件。");
      }
      const imported = await importProviderPlugin(manifest);
      await runtimeObserver.refresh();
      providerImportNotice = imported.loaded ? `${imported.displayName} 已导入并加载。`
        : `${imported.displayName} 已导入。${imported.diagnostic ?? "此 Provider 在 manifest 中被禁用。"}`;
    } catch (currentError) {
      providerOperationError = `导入 Provider 失败：${String(currentError)}`;
    } finally {
      providerImportBusy = false;
      await tick();
      if (providerOperationError) providerErrorElement?.focus();
    }
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

  async function deleteRemoteDevice(device: RemoteDevice) {
    if (device.status !== "revoked" || revokingRemoteCredentialId) return;
    const confirmed = await confirmDialog(`删除 ${device.deviceName} 的已撤销配对记录？删除后将不再显示在设备列表中。`, {
      title: "删除设备记录",
      kind: "warning",
    });
    if (!confirmed || revokingRemoteCredentialId) return;
    revokingRemoteCredentialId = device.id;
    remoteCommandError = null;
    try {
      await deleteRemoteClient(device.id);
      remoteDevices = remoteDevices.filter((candidate) => candidate.id !== device.id);
      applyRemoteClientSnapshot(await listRemoteClients());
      remoteClientsUnavailable = false;
    } catch (currentError) {
      remoteCommandError = remoteCommandDiagnostic(currentError, "remote_client_delete_failed");
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

  async function persistSettings(nextSettings: AppSettings) {
    settings = nextSettings;
    await saveSettings();
  }

  async function saveSettings() {
    if (!settings) return;
    pendingSettingsSaves += 1;
    try {
      normalizeSettings(settings);
      syncSelectedPetProfile();
      settings = await updateAppSettings(settings);
      petLibrary = {
        dataDirectory: petLibrary?.dataDirectory ?? settings.petLibrary.dataDirectory ?? "",
        selectedPetId: settings.petLibrary.selectedPetId,
        pets: settings.petLibrary.pets,
      };
    } finally { pendingSettingsSaves -= 1; }
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
      updateMessage = "正在检查更新…";
    }

    try {
      const update = await checkAppUpdate();
      if (!shouldPromptForUpdate(update, mode, settings)) {
        if (mode === "manual") {
          updateMessage = "当前已是最新版本。";
        }
        return;
      }

      availableUpdate = update;
      updatePromptMode = mode;
      updateMessage = `发现新版本 ${update.version}。`;
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
    updateMessage = updatePromptMode === "manual" ? `已忽略版本 ${update.version}。` : "";
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
    const manualDownload = availableUpdate.manualDownload === true;
    updateMessage = manualDownload ? "正在打开安装包下载…" : `正在安装 ${availableUpdate.version}…`;
    try {
      await installAppUpdate();
      if (manualDownload) {
        availableUpdate = null;
        updateInstallBusy = false;
        updateMessage = "已在浏览器打开 DMG 下载，下载后请手动安装。";
      }
    } catch (currentError) {
      updateInstallBusy = false;
      updateError = String(currentError);
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
      petLibrary = await importPetImage(sourcePath, filename);
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
      petLibrary = await setPathManagerPets(selected);
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
      settings = normalizeSettings(await setPathManagerData(path));
      managedPaths = await getAppPaths();
      appDataDir = managedPaths.data;
      petLibrary = await listPets();
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
    selected.imagePath = settings.pet.imagePath;
  }

  async function importAnimatedPet() {
    const selected = await open({ multiple: false, filters: [{ name: "Pet manifest", extensions: ["json"] }] });
    if (typeof selected !== "string") return;
    busyPet = "package";
    error = "";
    try {
      petLibrary = await importPetPackage(selected);
      settings = normalizeSettings(await getAppSettings());
    } catch (currentError) { error = String(currentError); }
    finally { busyPet = ""; }
  }

  async function savePetScale(scale: number) {
    if (!settings) return;
    settings.pet.scale = Math.min(5, Math.max(2, scale));
    await saveSettings();
  }

  $: latest = events.at(-1);
  $: recentVisibleEvents = events.slice(-5).reverse();
  $: enabledSources = petSources.filter(source => source.enabled);
  $: receivingSources = petSources.filter(source => source.connected);
  $: pageTitle = routeTitle(tab);
  $: appTheme = themeClassNames(settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "dark" : "light");
</script>

<main class={`app-shell main-theme ${appTheme}`} class:sidebar-collapsed={sidebarCollapsed} class:has-side-panel-column={!!$sidePanelCurrent && $sidePanelMode === "column"} inert={$webcontentReloading}>
  <WindowToolbar bind:collapsed={sidebarCollapsed} canGoBack={$navigation.canGoBack} canGoForward={$navigation.canGoForward} onHome={() => navigation.navigate("tasks")} onBack={navigation.back} onForward={navigation.forward} settingsActive={isSettingsRoute(tab)} onSettings={() => navigation.navigate("settings")} onError={(message) => error = message} />
  <aside id="main-sidebar" class="sidebar" inert={sidebarCollapsed}>
    <MainNavigation current={tab} onNavigate={navigation.navigate} />
  </aside>

  <section class="content-pane" class:history-workbench={tab === "tasks" || tab === "extraction"}>
    {#if tab !== "tasks" && tab !== "extraction"}<header class="topbar">
      <div>
        <div class="page-heading-row"><h2 bind:this={pageHeading} tabindex="-1">{pageTitle}</h2></div>
        {#if error}<p class="error">{error}</p>{/if}
      </div>
    </header>{/if}

    <div class="content" bind:this={contentElement}>
    {#if isConnectionRoute(tab)}
      <HorizontalTabs items={connectionRoutes} value={tab} label="连接接入分区" panelId="connection-panel" onChange={navigation.navigate} />
    {/if}
    <!-- The panel is focusable only while it has the tabpanel role. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div id={isConnectionRoute(tab) ? "connection-panel" : undefined} role={isConnectionRoute(tab) ? "tabpanel" : undefined} aria-labelledby={isConnectionRoute(tab) ? `tab-${tab}` : undefined} tabindex={isConnectionRoute(tab) ? 0 : undefined}>
    {#if appearanceVisited && settings}
      <div hidden={tab !== "appearance"}><AppearanceSettings {settings} bind:pending={appearancePending} onChange={(next) => settings = next} onSave={persistSettings} /></div>
    {/if}
    {#if tab === "tasks" || tab === "extraction"}
      <CodexWorkbench />
    {:else if tab === "connections"}
      <div class="agent-workspace">
        <section class="overview-grid" aria-label="运行概览">
          <article class="overview-card pixel-panel">
            <span><PlugZap size={17} /> 活动来源</span>
            <strong>{enabledSources.length}/{petSources.length}</strong>
            <p>已启用的通知来源</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><Activity size={17} /> 接收状态</span>
            <strong>{receivingSources.length}</strong>
            <p>已连接的通知来源</p>
          </article>
          <article class="overview-card pixel-panel">
            <span><ShieldAlert size={17} /> 桌宠任务列表</span>
            <strong>按来源提供</strong>
            <p>支持来源启用的回复、激活与授权动作</p>
          </article>
        </section>

        <section class="agent-section pixel-panel">
          <header class="section-head">
            <div>
              <span class="agent-kicker">NOTICE SOURCES</span>
              <h3>接入状态</h3>
            </div>
            <span>智能体与通知助手</span>
          </header>

          <PetSources bind:sources={petSources} />
        </section>
      </div>
    {:else if tab === "devices"}
      <div class="connection-workspace">
        <section class="device-section pixel-panel">
          <header class="section-head connection-section-head">
            <div>
              <span class="agent-kicker">PAIRED DEVICES</span>
              <h3>设备</h3>
              <p>管理已配对的 Remote 客户端及其访问权限。</p>
            </div>
            <Button bind:ref={addDeviceButton} class="connection-primary-button" type="button" disabled={remoteRuntimeStatus?.phase !== "available" || pairDeviceDialogOpen} onclick={openPairDeviceDialog}>
              <Plus size={17} /> 添加设备
            </Button>
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
                <Button type="button" disabled={remoteRetryBusy} onclick={retryRemoteAccessRuntime}>
                  <RefreshCw size={16} /> {remoteRetryBusy ? "重试中" : "重试"}
                </Button>
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
            onDelete={deleteRemoteDevice}
          />
        </section>

      </div>
    {:else if tab === "runtimes"}
      <div class="connection-workspace">
        <section class="runtime-section pixel-panel">
          {#if providerOperationError}<p bind:this={providerErrorElement} class="runtime-operation-error" role="alert" tabindex="-1">{providerOperationError}</p>{/if}
          {#if providerImportNotice}<p class="runtime-operation-notice" role="status">{providerImportNotice}</p>{/if}
          <div class="runtime-group">
            <h3 class="runtime-group-title">内置 Provider</h3>
            <div class="runtime-list">
              {#each agentRuntimes.filter((runtime) => isBuiltinProvider(runtime.providerId) && shouldShowRuntime(runtime)) as runtime (runtime.providerId)}
                <ProviderRuntimeInfo {runtime} connection={providerConnections.find((state) => (state.pluginId ?? state.providerId) === runtime.providerId)} connectionsLoaded={providerConnectionsLoaded} busy={busyRuntime === "all" || busyRuntime === runtime.providerId} onSelect={selectInstalledRuntime} />
              {:else}
                <p class="runtime-empty">{agentRuntimes.length ? "未检测到已安装的运行时" : "运行时状态尚未加载"}</p>
              {/each}
            </div>
            <div class="runtime-actions">
              <Button type="button" disabled={busyRuntime !== null || providerImportBusy} onclick={refreshRuntimes}><RefreshCw size={16} /> {busyRuntime === "all" ? "检测中…" : "重新检测"}</Button>
            </div>
          </div>
          {#if agentRuntimes.some((runtime) => !isBuiltinProvider(runtime.providerId))}
            <div class="runtime-group">
              <h3 class="runtime-group-title">自定义 Provider</h3>
              <div class="runtime-list">
                {#each agentRuntimes.filter((runtime) => !isBuiltinProvider(runtime.providerId)) as runtime (runtime.providerId)}
                  <ProviderRuntimeInfo {runtime} connection={providerConnections.find((state) => (state.pluginId ?? state.providerId) === runtime.providerId)} connectionsLoaded={providerConnectionsLoaded} busy={providerImportBusy || busyRuntime === "all" || busyRuntime === runtime.providerId} onSelect={selectInstalledRuntime} onRemove={removeCustomProvider} />
                {/each}
              </div>
            </div>
          {/if}
          <div class="runtime-custom-provider">
            <h3>添加自定义 Provider</h3>
            <Button bind:ref={providerAddButton} class="runtime-add-provider" type="button" aria-label="添加自定义 Provider" title="选择 Provider manifest" disabled={providerImportBusy} onclick={addCustomProvider}><Plus size={24} aria-hidden="true" /></Button>
          </div>
        </section>
      </div>
    {:else if tab === "usage"}
      <UsagePanel onConnections={() => navigation.navigate("runtimes")} />
    {:else if tab === "personalize" && settings}
      <div class="settings-workspace pet-creation">
        <PetLibraryPanel {settings} library={petLibrary} busy={busyPet} onSelect={activatePet} onRemove={removePet} onImportImage={importImagePet} onImportPackage={importAnimatedPet} onScale={savePetScale} />
        <Button class="context-link" onclick={() => navigation.navigate("appearance")}>调整主题、透明度与桌宠效果</Button>
      </div>
    {:else if tab === "appearance" && settings}
      <!-- AppearanceSettings remains mounted below while a delayed save settles. -->
    {:else if tab === "settings" && settings}
      <div class="settings-workspace">
        <SignalingServiceSettings value={settings.connection?.signalingService ?? null}
          onSave={async (address) => {
            if (!settings) throw new Error("设置尚未加载");
            const updated = await configureSignalingService(address);
            if (!updated.connection) throw new Error("请更新桌面 App 后再配置信令服务");
            settings = updated;
            return settings.connection?.signalingService ?? null;
          }} />
        <section class="appearance-editor pixel-panel"><h3>启动</h3><div class="settings-card">
            <label class="check">
              <Switch checked={launchAtLogin} disabled={busyLaunchAtLogin} onCheckedChange={toggleLaunchAtLogin} />
              开机自启动
            </label>
        </div></section>
          <section class="appearance-editor pixel-panel">
            <header class="panel-head">
              <h3><FolderCog size={18} /> 数据与存储</h3>
            </header><div class="settings-card">
            {#if managedPaths}
              <SettingsRow label="安装目录"><span class="path">{managedPaths.install}</span></SettingsRow>
              <SettingsRow label="工作目录" description="任务执行目录独立保存，更换数据目录不会移动工作目录。"><span class="path">{managedPaths.workspace}</span></SettingsRow>
            {/if}
            <SettingsRow label="数据目录" description={appDataRestartPending ? "已保存路径配置，重启后生效。" : "更改只切换数据目录，不复制或清空已有数据，保存后请重启。"}>
              <span class="path">{managedPaths?.data ?? "正在读取路径…"}</span>
              <div class="directory-actions">
                <Button disabled={busyAppDataDirectory} onclick={chooseAppDataDirectory}>修改</Button>
                <Button disabled={busyAppDataDirectory || !settings.data.dataDirectory} onclick={resetAppDataDirectory} aria-label="恢复默认数据目录">默认</Button>
              </div>
            </SettingsRow>
            <SettingsRow label="宠物资源目录">
              <span class="path">{petLibrary?.dataDirectory ?? settings.petLibrary.dataDirectory ?? "正在读取路径…"}</span>
              <Button disabled={busyPet === "directory"} onclick={choosePetDataDirectory}>修改</Button>
            </SettingsRow>
          </div></section>

      </div>
    {:else if tab === "notifications" && settings}
      <NotificationSettings {settings} bind:busyRobotChannel onSave={persistSettings} />
    {:else if tab === "permissions"}
      <PermissionSettings />
    {:else if tab === "about"}
      <AboutSettings checking={!!updateCheckMode || updateInstallBusy} message={updateMessage} error={updateError} ignoredVersion={settings?.updates.ignoredVersion ?? null} onCheck={() => checkForUpdates("manual")}
        reloaded={resourceReloaded} onExtraction={() => navigation.navigate("extraction")}
        reloadBlocked={!!(pendingSettingsSaves || appearancePending || busyRuntime || busyPet || busyAppDataDirectory || busyLaunchAtLogin || busyRobotChannel || updateInstallBusy || availableUpdate || pairDeviceDialogOpen)} />
    {:else if tab === "events"}
      <p class="section-description">事件日志：查看活动来源与运行时事件，排查接入问题。</p>
      <EventJournal />
    {/if}
    </div>
    </div>
  </section>

  <AppSidePanelHost />

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
<UpdateDialog update={availableUpdate} busy={updateInstallBusy} error={updateError} onDismiss={dismissAvailableUpdate} onInstall={installAvailableUpdate} />
</main>
{#if $webcontentReloading}<div class={`webcontent-reload-overlay main-theme ${appTheme}`} role="status" aria-live="polite">正在校验并加载前端资源…</div>{/if}
