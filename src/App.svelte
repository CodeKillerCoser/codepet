<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    Activity,
    Bell,
    Bot,
    Check,
    Clock3,
    FolderCog,
    FolderOpen,
    ImagePlus,
    Moon,
    Palette,
    PlugZap,
    Power,
    ShieldAlert,
    Sun,
    Trash2,
    Volume2,
  } from "@lucide/svelte";
  import { onMount } from "svelte";
  import { collectorEndpoint, deletePet, getAppSettings, importPetImage, listAgents, listPets, recentEvents, selectPet, setAgentEnabled, setPetDataDirectory, updateAppSettings } from "./lib/api";
  import { mergeEventFeed } from "./lib/eventFeed";
  import PetAvatar from "./lib/PetAvatar.svelte";
  import { playNotificationSound } from "./lib/sound";
  import type { AgentView, AppSettings, PetEvent, PetLibraryView } from "./lib/types";

  let tab: "agents" | "personalize" | "events" = "agents";
  let agents: AgentView[] = [];
  let settings: AppSettings | null = null;
  let petLibrary: PetLibraryView | null = null;
  let events: PetEvent[] = [];
  let endpoint = "";
  let busyAgent: string | null = null;
  let busyPet = "";
  let error = "";
  let systemDark = false;
  let eventPollTimer: number | null = null;

  onMount(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    systemDark = media.matches;
    const syncTheme = () => {
      systemDark = media.matches;
    };
    media.addEventListener("change", syncTheme);

    let disposed = false;
    let unlistenPetEvent: (() => void) | null = null;
    void (async () => {
      await keepWindowVisible();
      unlistenPetEvent = await listen<PetEvent>("pet-event", (event) => {
        events = mergeEventFeed(events, [event.payload]);
      });
      if (disposed) {
        unlistenPetEvent();
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
      clearEventPoll();
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
    try {
      const [nextAgents, nextEvents, nextEndpoint, nextPetLibrary] = await Promise.all([
        listAgents(),
        recentEvents(),
        collectorEndpoint(),
        listPets(),
      ]);
      agents = nextAgents;
      events = mergeEventFeed(events, nextEvents);
      endpoint = nextEndpoint;
      petLibrary = nextPetLibrary;
      settings = await getAppSettings();
    } catch (currentError) {
      error = String(currentError);
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

  async function saveSettings() {
    if (!settings) return;
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

  async function importImagePet() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp"] }],
    });
    if (typeof selected !== "string") return;

    busyPet = "import";
    error = "";
    try {
      const filename = selected.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || "Imported Pet";
      petLibrary = await importPetImage(selected, filename);
      settings = await getAppSettings();
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function choosePetDataDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;

    busyPet = "directory";
    error = "";
    try {
      petLibrary = await setPetDataDirectory(selected);
      settings = await getAppSettings();
    } catch (currentError) {
      error = String(currentError);
    } finally {
      busyPet = "";
    }
  }

  async function activatePet(petId: string) {
    busyPet = petId;
    error = "";
    try {
      petLibrary = await selectPet(petId);
      settings = await getAppSettings();
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
      settings = await getAppSettings();
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

  $: latest = events.at(-1);
  $: recentVisibleEvents = events.slice(-5).reverse();
  $: enabledAgents = agents.filter((agent) => agent.enabled);
  $: enabledHookCount = enabledAgents.reduce((count, agent) => count + agent.hookEvents.length, 0);
  $: pageTitle = tab === "agents" ? "Agent" : tab === "personalize" ? "个性化" : "最新事件";
  $: appTheme = settings?.appearance.theme === "dark" || (settings?.appearance.theme === "system" && systemDark) ? "theme-dark" : "theme-light";
</script>

<main class={`app-shell pixel-shell ${appTheme}`}>
  <aside class="sidebar pixel-panel">
    <nav class="tabs" aria-label="Code Pet settings">
      <button class:active={tab === "agents"} on:click={() => (tab = "agents")} aria-label="Agent 列表">
        <Bot size={18} /> Agent
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
                    <dd>{agent.hookEvents.length} 个 hooks</dd>
                  </div>
                </dl>
                <div class="event-row">
                  {#each agent.hookEvents as event}
                    <span>{event}</span>
                  {/each}
                </div>
                <div class="agent-controls">
                  <span class:online={agent.enabled} class="status-chip">{agent.enabled ? "已启用" : "未启用"}</span>
                  <button
                    class:enabled={agent.enabled}
                    class="power-button"
                    disabled={busyAgent === agent.id}
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
            <button class="preview-import-button" disabled={busyPet === "import"} on:click={importImagePet} aria-label="导入图片宠物">
              <ImagePlus size={18} />
            </button>
          </div>
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

          <section class="sound-editor pixel-panel">
            <header class="panel-head">
              <div>
                <h3><Bell size={18} /> 通知声音</h3>
              </div>
            </header>
            <div class="sound-summary">
              <strong>{soundLabel(settings.notifications.sound)}</strong>
              <span>{settings.notifications.ringOnPermission ? "授权时会响铃" : "授权提醒静音"} · {settings.notifications.ringOnFailure ? "失败时会响铃" : "失败提醒静音"}</span>
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
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.ringOnPermission} on:change={saveSettings} />
              授权时响铃
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={settings.notifications.ringOnFailure} on:change={saveSettings} />
              失败时响铃
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
