// Node adapter for the same bootstrap JSON consumed by the Rust PathManager.
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join } from "node:path";
export function getPaths({home = homedir(), platform = process.platform, env = process.env} = {}) {
  const base = platform === "win32" ? env.LOCALAPPDATA : platform === "darwin" ? join(home, "Library", "Application Support") : env.XDG_DATA_HOME || join(home, ".local", "share");
  if (!base || !isAbsolute(base)) throw new Error("Platform data directory unavailable");
  const defaultData = join(base, "code-pet");
  const settingsFile = env.CODE_PET_SETTINGS_PATH || join(defaultData, "config", "settings.json");
  const settings = existsSync(settingsFile) ? JSON.parse(readFileSync(settingsFile, "utf8")) : {};
  const data = settings.data?.dataDirectory?.trim() || defaultData;
  if (!isAbsolute(data)) throw new Error("Data directory must be absolute");
  return { data, workspace: join(home, ".codepet"), settingsFile, spool: join(data, "connections", "spool", "events.jsonl") };
}
