export type AgentId = "codex" | "claude" | "qoder";

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
}

export interface PixelPetSprite {
  body: string;
  accent: string;
  eyes: string;
}

export type PetKind = "palette" | "image" | "codex-atlas";

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

export interface AppSettings {
  appearance: {
    theme: "system" | "light" | "dark";
  };
  pet: {
    selectedPetId: string;
    kind: PetKind;
    sprite: PixelPetSprite;
    imagePath?: string | null;
    scale: number;
    alwaysOnTop: boolean;
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
