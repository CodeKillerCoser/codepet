import { invoke } from '@tauri-apps/api/core';

export interface DrawerPointer { x: number; y: number; pressed?: boolean; externalReleases?: number; releasePoint?: { x: number; y: number } }

export interface DrawerRect { x: number; y: number; width: number; height: number }
export interface DrawerPresentation { width: number; height: number; visible: DrawerRect[] }

export interface DrawerWindow {
  id: string;
  name: string;
  title: string;
  icon?: string;
  requiredWidth?: number;
  requiredHeight?: number;
}

export const defaultDrawerSize = { width: 800, height: 560 } as const;
export type DrawerSelectionOperation = 'drop' | 'dragEnd' | 'dockClick';

export function drawerCommand(operation: 'supported' | 'dragProbe' | 'park' | 'reset' | 'permission'): Promise<boolean>;
export function drawerCommand(operation: 'settings'): Promise<null>;
export function drawerCommand(operation: 'release', id: string): Promise<boolean>;
export function drawerCommand(operation: DrawerSelectionOperation): Promise<DrawerWindow | null>;
export function drawerCommand(operation: 'activate' | 'show', id: string, region: DrawerRect): Promise<DrawerPresentation | {pending: true}>;
export function drawerCommand(operation: string, id?: string, region?: DrawerRect) {
  return invoke('pet_window_drawer', {
    operation, id,
    region: region && { x: region.x, y: region.y, width: region.width, height: region.height },
  });
}

// Preserve command order across component teardown and remount.
let pending: Promise<unknown> = Promise.resolve();
export function serializeDrawer<T>(work: () => Promise<T>): Promise<T> {
  const next = pending.then(work);
  pending = next.catch(() => {});
  return next;
}
