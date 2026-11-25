import type { ActivityKeywordFilterSettings, AgentId, AppSettings, RobotNotificationChannel } from "../types";
import { defaultRunningBubbleSettings, normalizeThemePreset } from "../theme";

export const agentOrder: AgentId[] = ["codex", "claude", "qoder", "cursor"];
export const runningBubbleDefaults = defaultRunningBubbleSettings;
export const agentActivityFilterDefaults: ActivityKeywordFilterSettings = {
  titleKeywords: [],
  messageKeywords: [],
};
export const activityFilterDefaults: AppSettings["activityFilters"] = {
  titleKeywords: [],
  messageKeywords: [],
  byAgent: {},
};
export const agentRuntimeSettingsDefaults: AppSettings["agentRuntimes"] = {
  byProvider: {},
};
export const robotNotificationDefaults: AppSettings["notifications"]["robot"] = {
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
export const defaultPetOpacity = 1;
export const minPetOpacity = 0.25;

export function normalizeSettings(nextSettings: AppSettings) {
  nextSettings.data = {
    ...(nextSettings.data ?? {}),
  };
  nextSettings.appearance.preset = normalizeThemePreset(nextSettings.appearance.preset);
  nextSettings.appearance.runningBubble = {
    ...runningBubbleDefaults,
    ...(nextSettings.appearance.runningBubble ?? {}),
  };
  nextSettings.appearance.runningBubble.animationMs = clampRunningBubbleAnimationMs(nextSettings.appearance.runningBubble.animationMs);
  nextSettings.appearance.runningBubble.borderWidth = clampRunningBubbleBorderWidth(nextSettings.appearance.runningBubble.borderWidth);
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

export function normalizeRobotNotificationSettings(robot: Partial<AppSettings["notifications"]["robot"]> | null | undefined): AppSettings["notifications"]["robot"] {
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

export function normalizeRobotTemplate(template: Partial<AppSettings["notifications"]["robot"]["template"]> | null | undefined): AppSettings["notifications"]["robot"]["template"] {
  return {
    ...robotNotificationDefaults.template,
    ...(template ?? {}),
  };
}

export function normalizeRobotChannel(channel: Partial<RobotNotificationChannel> | null | undefined): RobotNotificationChannel | null {
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

export function robotChannelId(provider: RobotNotificationChannel["provider"]) {
  return `${provider}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

export function createRobotChannel(provider: RobotNotificationChannel["provider"]): RobotNotificationChannel {
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

export function normalizeRobotList(values: Array<string | null | undefined> | null | undefined): string[] {
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

export function normalizeActivityFilters(filters: Partial<AppSettings["activityFilters"]> | null | undefined): AppSettings["activityFilters"] {
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

export function normalizeAgentActivityFilters(filters: Partial<ActivityKeywordFilterSettings> | null | undefined): ActivityKeywordFilterSettings {
  return {
    titleKeywords: normalizeFilterKeywords(filters?.titleKeywords),
    messageKeywords: normalizeFilterKeywords(filters?.messageKeywords),
  };
}

export function normalizeAgentSettings(agentSettings: Partial<AppSettings["agents"]> | null | undefined): AppSettings["agents"] {
  return { byAgent: {} };
}

export function normalizeFilterKeywords(keywords: string[] | null | undefined): string[] {
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

export function clampPetOpacity(value: number | null | undefined) {
  const numericValue = Number(value);
  if (!Number.isFinite(numericValue)) {
    return defaultPetOpacity;
  }
  return Math.min(defaultPetOpacity, Math.max(minPetOpacity, numericValue));
}

export function clampRunningBubbleAnimationMs(value: number) {
  return Math.min(4000, Math.max(600, Math.round(value || runningBubbleDefaults.animationMs)));
}

export function clampRunningBubbleBorderWidth(value: number) {
  return Math.min(8, Math.max(1, Math.round(value || runningBubbleDefaults.borderWidth)));
}
