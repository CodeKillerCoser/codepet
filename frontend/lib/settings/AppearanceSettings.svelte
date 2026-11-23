<script lang="ts">
  import { Button } from "../ui/button/index";
  import { Switch } from "../ui/switch/index";
  import { FolderOpen, Moon, Sun, Volume2 } from "@lucide/svelte";
  import { onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import SettingsRow from "../SettingsRow.svelte";
  import { updateAppSettings } from "../api";
  import type { AppSettings } from "../types";
  import { playWhipReactionSound } from "../sound";
  import { defaultRunningBubbleSettings } from "../theme";
  import { colorStopIndexFromBand, updateRunningBubbleColorSetting, type RunningBubbleColorKey } from "../bubbleColorSettings";
  import { gradientEditorFromCss, gradientSegmentCss, nextGradientStopColor, type GradientEditorValue } from "../gradientColor";
  import { normalizeSettings, clampPetOpacity, clampRunningBubbleAnimationMs, clampRunningBubbleBorderWidth, minPetOpacity } from "./model";
  export let settings: AppSettings;
  export let onSave: (settings: AppSettings) => Promise<void>;
  export let onChange: (settings: AppSettings) => void;
  export let pending = false;
  let error = "", inFlight = 0;
  let runningBubbleSaveToken = 0;
  let runningBubbleSaveTimer: number | null = null;
  let selectedBubbleColorStop: Record<RunningBubbleColorKey, number> = { backgroundColor: 0, borderColor: 0 };
  const runningBubbleDefaults = defaultRunningBubbleSettings;
  $: pending = runningBubbleSaveTimer !== null || inFlight > 0;
  onDestroy(() => { runningBubbleSaveToken += 1; clearRunningBubbleSaveTimer(); });
  async function saveSettings() { await onSave(settings); }
  const whipReactionSounds: Array<{ value: AppSettings["pet"]["whipReactionSound"]; label: string }> = [
    { value: "none", label: "无" },
    { value: "pa", label: "啪" },
    { value: "scream", label: "啊啊啊" },
    { value: "custom", label: "自定义" },
  ];
  const bubbleColorConfigs = [
    { key: "backgroundColor", label: "背景色", fallback: runningBubbleDefaults.backgroundColor, directional: true },
    { key: "borderColor", label: "边框色", fallback: runningBubbleDefaults.borderColor, directional: false },
  ] as const;
  async function setTheme(theme: AppSettings["appearance"]["theme"]) {
    if (!settings) return;
    settings.appearance.theme = theme;
    await saveSettings();
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

  function petOpacityLabel(value: number | null | undefined) {
    return `${Math.round(clampPetOpacity(value) * 100)}%`;
  }

  function runningBubbleSpeedLabel(value: number) {
    return `${(clampRunningBubbleAnimationMs(value) / 1000).toFixed(1)}s`;
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
    onChange(settings);
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

    inFlight += 1;
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
      onChange(settings);
    } catch (currentError) {
      if (saveToken === runningBubbleSaveToken) {
        error = String(currentError);
      }
    } finally { inFlight -= 1; }
  }

  async function savePetOpacity() {
    if (!settings) return;
    settings.pet.opacity = clampPetOpacity(settings.pet.opacity);
    await saveSettings();
  }
</script>
{#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="settings-workspace">
          <section class="appearance-editor pixel-panel">
            <header class="panel-head">
              <h3>主题</h3>
            </header><div class="settings-card">
            <SettingsRow label="主题模式"><section class="theme-switcher" aria-label="主题模式">
              <Button class={[(settings.appearance.theme === "light") ? "active" : ''].filter(Boolean).join(' ')}  onclick={() => setTheme("light")} aria-label="浅色模式">
                <Sun size={16} /> Light
              </Button>
              <Button class={[(settings.appearance.theme === "dark") ? "active" : ''].filter(Boolean).join(' ')}  onclick={() => setTheme("dark")} aria-label="深色模式">
                <Moon size={16} /> Dark
              </Button>
              <Button class={[(settings.appearance.theme === "system") ? "active" : ''].filter(Boolean).join(' ')}  onclick={() => setTheme("system")} aria-label="跟随系统">
                Auto
              </Button>
            </section></SettingsRow>
          </div></section>

          <section class="bubble-editor pixel-panel">
            <header class="panel-head">
              <h3>任务气泡</h3>
            </header><div class="settings-card">
            <div class="bubble-toggle-grid">
              <label class="check">
                <Switch bind:checked={settings.appearance.runningBubble.backgroundBreathing} onCheckedChange={saveRunningBubbleSettings} />
                背景色呼吸灯
              </label>
              <label class="check">
                <Switch bind:checked={settings.appearance.runningBubble.borderMarquee} onCheckedChange={saveRunningBubbleSettings} />
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
                  <Button
                    class="color-band-preview"
                    type="button"
                    aria-label={`选择${colorConfig.label}色段`}
                    style={`background: ${gradientSegmentCss(editor.colors)}`}
                    onclick={(event) => selectBubbleColorStopFromBand(colorConfig.key, editor.colors.length, event)}
                  ></Button>
                  <div class="color-stop-editor" aria-label={`${colorConfig.label}当前颜色`}>
                    <Button type="button" aria-label="上一个颜色" disabled={selectedColorIndex === 0} onclick={() => selectPreviousBubbleColorStop(colorConfig.key, editor.colors.length)}>‹</Button>
                    <label class="color-stop">
                      {#key `${colorConfig.key}-${selectedColorIndex}-${editor.colors[selectedColorIndex]}`}
                        <input type="color" value={editor.colors[selectedColorIndex]} on:input={(event) => updateBubbleColorStop(colorConfig.key, colorConfig.fallback, selectedColorIndex, inputValue(event))} />
                      {/key}
                    </label>
                    <Button type="button" aria-label="下一个颜色" disabled={selectedColorIndex >= editor.colors.length - 1} onclick={() => selectNextBubbleColorStop(colorConfig.key, editor.colors.length)}>›</Button>
                    <span>{selectedColorIndex + 1}/{editor.colors.length}</span>
                    <Button class="add-color-stop" type="button" onclick={() => addBubbleColorStop(colorConfig.key, colorConfig.fallback)}>添加颜色</Button>
                    {#if editor.colors.length > 1}
                      <Button class="remove-color-stop" type="button" aria-label={`移除当前 ${colorConfig.label}颜色`} onclick={() => removeBubbleColorStop(colorConfig.key, colorConfig.fallback, selectedColorIndex)}>移除</Button>
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
          </div></section>


        <section class="appearance-editor pixel-panel"><h3>窗口与互动</h3><div class="settings-card">
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
            <SettingsRow label="互动反应音"><div class="settings-control-stack">
            <div class="segmented">
              {#each whipReactionSounds as reaction}
                <Button class={[(settings.pet.whipReactionSound === reaction.value) ? "active" : ''].filter(Boolean).join(' ')}

                  onclick={async () => {
                    settings.pet.whipReactionSound = reaction.value;
                    await saveSettings();
                  }}
                >
                  {reaction.label}
                </Button>
              {/each}
            </div>
            <div class="row-actions">
              <Button onclick={() => playWhipReactionSound(settings.pet.whipReactionSound, settings.pet.customWhipReactionSoundPath)}>
                <Volume2 size={17} /> 试听反应
              </Button>
              <Button onclick={pickCustomWhipReactionSound}>
                <FolderOpen size={17} /> 选择反应音频
              </Button>
            </div>
            {#if settings.pet.customWhipReactionSoundPath}
              <p class="path">{settings.pet.customWhipReactionSoundPath}</p>
            {/if}</div></SettingsRow>
        </div></section>
      </div>
