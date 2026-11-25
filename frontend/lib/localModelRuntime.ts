import { invoke } from "@tauri-apps/api/core";
export interface RuntimeInstallation {
  name: string;
  version: string;
  path: string;
  source: "system" | "managed";
}
export interface RuntimeJob {
  id: string;
  state: string;
  stage: string;
  downloadedBytes: number;
  totalBytes: number;
  version: string;
  canCancel: boolean;
  error: string | null;
}
export interface RuntimeStatus {
  supported: boolean;
  reason: string | null;
  installation: RuntimeInstallation | null;
  endpoint: string;
  job: RuntimeJob | null;
}
export interface RuntimePlan {
  id: string;
  version: string;
  bytes: number;
  sourceUrl: string;
}
export const runtimeRequest = <T = RuntimeStatus>(
  action = "status",
  id?: string,
): Promise<T> =>
  invoke("task_ledger_request", {
    request: { op: "local-model-runtime", action, id },
  });
export const runtimeActive = (job: RuntimeJob | null) =>
  !!job &&
  [
    "preparing",
    "downloading",
    "verifying",
    "installing",
    "cancelling",
  ].includes(job.state);
export function byteSize(bytes: number) {
  return bytes < 1024 * 1024
    ? `${Math.ceil(bytes / 1024)} KB`
    : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
