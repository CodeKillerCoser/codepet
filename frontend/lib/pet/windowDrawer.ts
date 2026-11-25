import { invoke } from '@tauri-apps/api/core';

export interface DrawerPointer { x: number; y: number; pressed?: boolean; externalReleases?: number; releasePoint?: { x: number; y: number } }

export interface DrawerRect { x: number; y: number; width: number; height: number }
export interface DrawerPresentation { width: number; height: number; visible: DrawerRect[]; awaitingSurface: boolean; resizing: boolean }

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
type DrawerSlot = DrawerRect & { maxWidth: number; maxHeight: number };

export function drawerCommand(operation: 'supported' | 'dragProbe' | 'reset' | 'permission'): Promise<boolean>;
export function drawerCommand(operation: 'park'): Promise<boolean>;
export function drawerCommand(operation: 'settings'): Promise<null>;
export function drawerCommand(operation: 'release', id: string): Promise<boolean>;
export function drawerCommand(operation: DrawerSelectionOperation): Promise<DrawerWindow | null>;
export function drawerCommand(operation: 'activate' | 'show', id: string, region: DrawerSlot): Promise<DrawerPresentation | {pending: true}>;
export function drawerCommand(operation: string, id?: string, region?: DrawerSlot) {
  return invoke('pet_window_drawer', {
    operation, id,
    region,
  });
}

// Preserve command order across component teardown and remount.
let pending: Promise<unknown> = Promise.resolve();
export function serializeDrawer<T>(work: () => Promise<T>): Promise<T> {
  const next = pending.then(work);
  pending = next.catch(() => {});
  return next;
}
