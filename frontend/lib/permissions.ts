import { invoke } from "@tauri-apps/api/core";

export interface SystemPermissionSettings {
  fullDiskAccessSupported: boolean;
  appPath: string | null;
}

export const getSystemPermissionSettings = () => invoke<SystemPermissionSettings>("get_system_permission_settings");
export const openFullDiskAccessSettings = () => invoke<void>("open_full_disk_access_settings");
export const revealPermissionApp = () => invoke<void>("reveal_permission_app");
