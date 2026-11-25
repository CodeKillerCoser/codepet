<script lang="ts">
  import { Button } from "../ui/button/index";
  import { Switch } from "../ui/switch/index";
  import { Input } from "../ui/input/index";
  import { Textarea } from "../ui/textarea/index";
  import { Bot, FolderOpen, PlugZap, Trash2, Volume2 } from "@lucide/svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import SettingsRow from "../SettingsRow.svelte";
  import { sendTestRobotNotification } from "../api";
  import { playNotificationSound } from "../sound";
  import type { AppSettings, RobotNotificationChannel, DingTalkRobotChannel } from "../types";
  import { normalizeRobotNotificationSettings, normalizeRobotTemplate, normalizeRobotList, createRobotChannel } from "./model";
  export let settings: AppSettings;
  export let onSave: (settings: AppSettings) => Promise<void>;
  export let busyRobotChannel = "";
  let robotNotificationResult = "", error = "";
  async function saveSettings() { await onSave(settings); }
  const inputValue = (event: Event) => (event.currentTarget as HTMLInputElement).value;
  const robotTemplateFields = [
    { key: "title", label: "标题", rows: 1 },
    { key: "header", label: "摘要", rows: 1 },
    { key: "primary", label: "一级内容", rows: 3 },
    { key: "secondary", label: "二级内容", rows: 3 },
    { key: "footer", label: "页脚", rows: 1 },
  ] as const;
  const robotTemplateVariables = "{{statusIcon}} {{status}} {{agent}} {{task}} {{contentBlock}} {{cwdLine}} {{toolLine}} {{sessionLine}} {{time}}";
  const notificationSounds: Array<{ value: AppSettings["notifications"]["sound"]; label: string }> = [
    { value: "blip", label: "轻滴" },
    { value: "chime", label: "清脆" },
    { value: "bell", label: "铃声" },
    { value: "custom", label: "自定义" },
    { value: "silent", label: "静音" },
  ];
  const robotTriggerOptions = [
    { key: "waitingApproval", label: "等待审批或输入" },
    { key: "taskFailed", label: "任务失败" },
    { key: "taskDone", label: "任务完成" },
  ] as const;
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
</script>
{#if error}<p class="error" role="alert">{error}</p>{/if}
      <div class="settings-workspace">
          <section class="sound-editor pixel-panel">
            <header class="panel-head">
              <div>
                <h3>通知声音</h3>
              </div>
            </header><div class="settings-card">
            <SettingsRow label="通知音色"><div class="settings-control-stack">
            <div class="segmented">
              {#each notificationSounds as sound}
                <Button class={[(settings.notifications.sound === sound.value) ? "active" : ''].filter(Boolean).join(' ')}
                  aria-pressed={settings.notifications.sound === sound.value}
                  onclick={async () => {
                    settings.notifications.sound = sound.value;
                    await saveSettings();
                  }}
                >
                  {sound.label}
                </Button>
              {/each}
            </div>
            <div class="row-actions">
              <Button onclick={() => playNotificationSound(settings)}>
                <Volume2 size={17} /> 试听
              </Button>
              <Button onclick={pickCustomSound}>
                <FolderOpen size={17} /> 选择音频
              </Button>
            </div>
            {#if settings.notifications.customSoundPath}
              <p class="path">{settings.notifications.customSoundPath}</p>
            {/if}</div></SettingsRow>
            <label class="check">
              <Switch bind:checked={settings.notifications.ringOnPermission} onCheckedChange={saveSettings} />
              授权时响铃
            </label>
            <label class="check">
              <Switch bind:checked={settings.notifications.ringOnFailure} onCheckedChange={saveSettings} />
              失败时响铃
            </label>
            <label class="check">
              <Switch bind:checked={settings.notifications.ringOnDone} onCheckedChange={saveSettings} />
              任务结束时响铃
            </label>
            <label>
              重复提醒（秒）
              <Input type="number" min="5" max="300" bind:value={settings.notifications.repeatSeconds} onchange={saveSettings} />
            </label>
            <label class="check">
              <Switch bind:checked={settings.notifications.quietHoursEnabled} onCheckedChange={saveSettings} />
              静音时段
            </label>
            <SettingsRow label="静音时间"><div class="time-row">
              <Input aria-label="静音开始时间" type="time" bind:value={settings.notifications.quietHoursStart} onchange={saveSettings} />
              <Input aria-label="静音结束时间" type="time" bind:value={settings.notifications.quietHoursEnd} onchange={saveSettings} />
            </div></SettingsRow>
          </div></section>

          <section class="robot-editor pixel-panel">
            <header class="panel-head">
              <div>
                <h3>通知机器人</h3>
              </div>
            </header><div class="settings-card">
              <label class="switch-check">
                <Switch bind:checked={settings.notifications.robot.enabled} onCheckedChange={saveSettings} />
                启用机器人通知
              </label>

            <div class="robot-trigger-grid" aria-label="机器人触发事件">
              {#each robotTriggerOptions as trigger}
                <label class="check">
                  <Switch bind:checked={settings.notifications.robot.triggers[trigger.key]} onCheckedChange={saveSettings} />
                  {trigger.label}
                </label>
              {/each}
            </div>

            <details class="robot-template-panel">
              <summary>自定义消息模板</summary>
              <div class="robot-template-head">
                <strong>消息模板</strong>
                <Button type="button" onclick={resetRobotTemplate}>恢复默认</Button>
              </div>
              <code>{robotTemplateVariables}</code>
              <div class="robot-template-grid">
                {#each robotTemplateFields as field}
                  <label>
                    {field.label}
                    <Textarea
                      rows={field.rows}
                      value={settings.notifications.robot.template[field.key]}
                      onchange={(event) => updateRobotTemplateField(field.key, inputValue(event))}
                    ></Textarea>
                  </label>
                {/each}
              </div>
            </details>

            <div class="row-actions">
              <Button onclick={() => addRobotChannel("dingtalk")}>
                <Bot size={17} /> 钉钉
              </Button>
              <Button onclick={() => addRobotChannel("feishu")}>
                <Bot size={17} /> 飞书
              </Button>
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
                        <Switch bind:checked={channel.enabled} onCheckedChange={saveSettings} />
                        启用
                      </label>
                    </div>

                    <label>
                      名称
                      <Input type="text" bind:value={channel.name} onchange={saveSettings} />
                    </label>

                    {#if channel.provider === "dingtalk"}
                      <div class="segmented">
                        <Button class={[(channel.authMode === "enterprise-robot") ? "active" : ''].filter(Boolean).join(' ')}

                          onclick={async () => {
                            channel.authMode = "enterprise-robot";
                            await saveSettings();
                          }}
                        >
                          企业机器人
                        </Button>
                        <Button class={[(channel.authMode === "webhook") ? "active" : ''].filter(Boolean).join(' ')}

                          onclick={async () => {
                            channel.authMode = "webhook";
                            await saveSettings();
                          }}
                        >
                          Webhook
                        </Button>
                      </div>

                      {#if channel.authMode === "enterprise-robot"}
                        <div class="robot-field-grid">
                          <label>
                            robotCode
                            <Input type="text" bind:value={channel.robotCode} onchange={saveSettings} />
                          </label>
                          <label>
                            clientId
                            <Input type="text" bind:value={channel.clientId} onchange={saveSettings} />
                          </label>
                        </div>
                        <label>
                          clientSecret
                          <Input type="password" bind:value={channel.clientSecret} onchange={saveSettings} />
                        </label>
                        <div class="segmented">
                          <Button class={[(channel.targetType === "user-ids") ? "active" : ''].filter(Boolean).join(' ')}

                            onclick={async () => {
                              channel.targetType = "user-ids";
                              await saveSettings();
                            }}
                          >
                            用户
                          </Button>
                          <Button class={[(channel.targetType === "open-conversation-id") ? "active" : ''].filter(Boolean).join(' ')}

                            onclick={async () => {
                              channel.targetType = "open-conversation-id";
                              await saveSettings();
                            }}
                          >
                            群
                          </Button>
                        </div>
                        {#if channel.targetType === "user-ids"}
                          <label>
                            userId
                            <Textarea rows={2} value={dingTalkUserIdsValue(channel)} onchange={(event) => {
                              updateDingTalkUserIds(channel, inputValue(event));
                              void saveSettings();
                            }}></Textarea>
                          </label>
                        {:else}
                          <label>
                            openConversationId
                            <Input type="text" bind:value={channel.openConversationId} onchange={saveSettings} />
                          </label>
                        {/if}
                      {:else}
                        <label>
                          webhook
                          <Input type="url" bind:value={channel.webhookUrl} onchange={saveSettings} />
                        </label>
                        <label>
                          加签密钥
                          <Input type="password" bind:value={channel.webhookSecret} onchange={saveSettings} />
                        </label>
                      {/if}
                    {:else}
                      <label>
                        webhook
                        <Input type="url" bind:value={channel.webhookUrl} onchange={saveSettings} />
                      </label>
                      <label>
                        签名密钥
                        <Input type="password" bind:value={channel.webhookSecret} onchange={saveSettings} />
                      </label>
                    {/if}

                    <div class="robot-channel-actions">
                      <Button disabled={busyRobotChannel === channel.id} onclick={() => testRobotChannel(channel.id)}>
                        <PlugZap size={17} /> 测试
                      </Button>
                      <Button class="danger-action" onclick={() => removeRobotChannel(channel.id)} aria-label={`删除 ${robotChannelLabel(channel)}`}>
                        <Trash2 size={17} /> 删除
                      </Button>
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
          </div></section>
      </div>
