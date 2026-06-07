export type AgentId = "codex" | "claude" | "qoder" | "cursor";

export type TaskStatus =
  | "idle"
  | "thinking"
  | "running"
  | "waiting-approval"
  | "failed"
  | "done";

export type PetEventKind =
  | "task-started"
  | "task-updated"
  | "tool-started"
  | "permission-requested"
  | "message"
  | "task-failed"
  | "task-completed";

export interface AgentView {
  id: AgentId;
  name: string;
  description: string;
  enabled: boolean;
  configPath: string;
  hookEvents: string[];
  selectedHookEvents: string[];
}

export interface PixelPetSprite {
  body: string;
  accent: string;
  eyes: string;
}

export type PetKind = "palette" | "image" | "codex-atlas";
export type WhipReactionSound = "none" | "pa" | "scream" | "custom";

export interface ConfiguredPet {
  id: string;
  name: string;
  kind: PetKind;
  sprite?: PixelPetSprite | null;
  imagePath?: string | null;
  sourcePath?: string | null;
  createdAt: string;
}

export interface PetLibraryView {
  dataDirectory: string;
  selectedPetId: string;
  pets: ConfiguredPet[];
}

export interface SubjectCutoutResult {
  sourcePath: string;
  outputPath: string;
  width: number;
  height: number;
  mimeType: "image/png";
}

export interface AppSettings {
  appearance: {
    theme: "system" | "light" | "dark";
    runningBubble: {
      backgroundBreathing: boolean;
      borderMarquee: boolean;
      backgroundColor: string;
      borderColor: string;
      borderWidth: number;
      animationMs: number;
    };
  };
  pet: {
    selectedPetId: string;
    kind: PetKind;
    sprite: PixelPetSprite;
    imagePath?: string | null;
    scale: number;
    imagePixelSize: number;
    opacity: number;
    alwaysOnTop: boolean;
    whipReactionSound: WhipReactionSound;
    customWhipReactionSoundPath?: string | null;
  };
  petLibrary: {
    dataDirectory?: string | null;
    selectedPetId: string;
    pets: ConfiguredPet[];
    deletedPetIds?: string[];
  };
  notifications: {
    sound: "blip" | "chime" | "bell" | "custom" | "silent";
    customSoundPath?: string | null;
    ringOnPermission: boolean;
    ringOnFailure: boolean;
    ringOnDone: boolean;
    repeatSeconds: number;
    quietHoursEnabled: boolean;
    quietHoursStart: string;
    quietHoursEnd: string;
  };
  activityFilters: ActivityFilterSettings;
  agents: AgentSettings;
}

export interface ActivityFilterSettings {
  titleKeywords: string[];
  messageKeywords: string[];
  byAgent: Partial<Record<AgentId, ActivityKeywordFilterSettings>>;
}

export interface ActivityKeywordFilterSettings {
  titleKeywords: string[];
  messageKeywords: string[];
}

export interface AgentSettings {
  byAgent: Partial<Record<AgentId, AgentPreferenceSettings>>;
}

export interface AgentPreferenceSettings {
  hookEvents: string[];
}

export interface PetEvent {
  id: string;
  provider: AgentId;
  kind: PetEventKind;
  status: TaskStatus;
  title: string;
  message: string;
  sessionId?: string | null;
  cwd?: string | null;
  toolName?: string | null;
  shouldRing: boolean;
  createdAt: string;
  endedAt?: string | null;
  raw: unknown;
  source?: ActivitySource | null;
}

export interface TokenUsage {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  cacheCreationInputTokens: number;
  cacheReadInputTokens: number;
  totalTokens: number;
}

export interface TokenUsageSummary {
  total: TokenUsage;
  byProvider: Array<{
    provider: AgentId;
    sessions: number;
    total: TokenUsage;
  }>;
  byDay: Array<{
    day: string;
    sessions: number;
    total: TokenUsage;
  }>;
  byBucket: Array<{
    provider: AgentId;
    bucketStart: string;
    sessions: number;
    total: TokenUsage;
  }>;
  sessions: Array<{
    provider: AgentId;
    sessionId: string;
    day: string;
    models: string[];
    usage: TokenUsage;
  }>;
}

export interface ActivitySource {
  pid?: number | null;
  ppid?: number | null;
  terminalProgram?: string | null;
  termSessionId?: string | null;
  ttyPath?: string | null;
  tmuxPane?: string | null;
  weztermPane?: string | null;
  kittyWindowId?: string | null;
  appBundleId?: string | null;
}
