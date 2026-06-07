import { invoke } from "@tauri-apps/api/core";
import type { AgentView, AppSettings, PetEvent, PetLibraryView, SubjectCutoutResult, TokenUsageSummary } from "./types";

export interface PerfEventPayload {
  name: string;
  durationMs: number;
  status?: "ok" | "error";
  fields?: Record<string, string | number | boolean | null>;
  error?: string;
}

export async function listAgents(): Promise<AgentView[]> {
  return invoke<AgentView[]>("list_agents");
}

export async function setAgentEnabled(agentId: string, enabled: boolean): Promise<AgentView[]> {
  return invoke<AgentView[]>("set_agent_enabled", { agentId, enabled });
}

export async function setAgentHookEvents(agentId: string, hookEvents: string[]): Promise<AgentView[]> {
  return invoke<AgentView[]>("set_agent_hook_events", { agentId, hookEvents });
}

export async function getAppSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_app_settings");
}

export async function updateAppSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("update_app_settings", { settings });
}

export async function getLaunchAtLoginEnabled(): Promise<boolean> {
  return invoke<boolean>("get_launch_at_login_enabled");
}

export async function setLaunchAtLoginEnabled(enabled: boolean): Promise<boolean> {
  return invoke<boolean>("set_launch_at_login_enabled", { enabled });
}

export async function listPets(): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("list_pets");
}

export async function selectPet(petId: string): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("select_pet", { petId });
}

export async function deletePet(petId: string): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("delete_pet", { petId });
}

export async function setPetDataDirectory(path: string): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("set_pet_data_directory", { path });
}

export async function importPetImage(sourcePath: string, name?: string, pixelSize?: number): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("import_pet_image", { sourcePath, name, pixelSize });
}

export async function updatePetImagePixelSize(pixelSize: number): Promise<PetLibraryView> {
  return invoke<PetLibraryView>("update_pet_image_pixel_size", { pixelSize });
}

export async function cutOutImageSubject(sourcePath: string, outputPath?: string): Promise<SubjectCutoutResult> {
  return invoke<SubjectCutoutResult>("cut_out_image_subject", { sourcePath, outputPath });
}

export async function recentEvents(): Promise<PetEvent[]> {
  let ipcEvents: PetEvent[] | null = null;
  try {
    ipcEvents = await withTimeout(invoke<PetEvent[]>("recent_events"), 1200);
    if (ipcEvents.length > 0) {
      return ipcEvents;
    }
  } catch {
    // Browser preview cannot use Tauri IPC, so fall back to the local collector HTTP endpoint.
  }

  try {
    const response = await withTimeout(fetch("http://127.0.0.1:47621/events"), 1200);
    if (response.ok) {
      return (await response.json()) as PetEvent[];
    }
  } catch {
    // The collector may still be starting.
  }
  return ipcEvents ?? [];
}

export async function tokenUsageSummary(): Promise<TokenUsageSummary> {
  return invoke<TokenUsageSummary>("token_usage_summary");
}

export async function recordPerfEvent(event: PerfEventPayload): Promise<void> {
  return invoke<void>("record_perf_event", { event });
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = globalThis.setTimeout(() => reject(new Error(`operation timed out after ${timeoutMs}ms`)), timeoutMs);
    promise.then(
      (value) => {
        globalThis.clearTimeout(timeout);
        resolve(value);
      },
      (error) => {
        globalThis.clearTimeout(timeout);
        reject(error);
      },
    );
  });
}

export async function collectorEndpoint(): Promise<string> {
  return invoke<string>("collector_endpoint");
}

export async function activateActivity(eventId: string): Promise<void> {
  return invoke<void>("activate_activity", { eventId });
}

export async function openMainWindow(): Promise<void> {
  return invoke<void>("open_main_window");
}

export async function sendActivityReply(eventId: string, message: string): Promise<void> {
  return invoke<void>("send_activity_reply", { eventId, message });
}

export async function resolveActivityApproval(eventId: string, behavior: "allow" | "deny", message?: string): Promise<void> {
  return invoke<void>("resolve_activity_approval", { eventId, behavior, message });
}
