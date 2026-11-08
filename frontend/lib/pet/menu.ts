import { invoke } from '@tauri-apps/api/core';
import { builtInPet, loadPetAsset } from './assets';
import { petStatusLabel, type PetTask } from '../petGateway';
import type { AppSettings } from '../types';

export function menuStatus(tasks: PetTask[], error: string) {
  return { label: error ? '连接中断' : tasks[0] ? petStatusLabel(tasks[0].status) : '就绪', count: tasks.length };
}

/** One static head per selected asset; do not stream animation frames into the menu bar. */
export class PetMenuController {
  private revision = 0;
  private assetKey = '';
  private portrait?: Promise<string | undefined>;
  private sent = '';
  async update(settings: AppSettings, tasks: PetTask[], error: string) {
    const revision = ++this.revision;
    const pet = settings.pet;
    const key = JSON.stringify([pet.selectedPetId, pet.kind, pet.imagePath]);
    if (key !== this.assetKey) {
      this.assetKey = key;
      this.portrait = head(pet).catch(() => head({ ...pet, selectedPetId: 'default', kind: 'builtin', imagePath: null })).catch(() => undefined);
    }
    const portrait = await this.portrait;
    if (revision !== this.revision) return;
    const summary = { ...menuStatus(tasks, error), portrait };
    const encoded = JSON.stringify(summary);
    if (encoded === this.sent) return;
    await invoke('pet_menu_summary', { summary });
    if (revision === this.revision) this.sent = encoded;
  }
  dispose() { this.revision++; }
}

async function head(pet: AppSettings['pet']): Promise<string> {
  const asset = await loadPetAsset(pet.selectedPetId ?? 'default', pet.kind, pet.imagePath);
  const image = new Image();
  image.src = asset.url || builtInPet('default').url;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = 36; canvas.height = 24;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('无法绘制菜单栏头像');
  const width = asset.atlas ? image.naturalWidth / 8 : image.naturalWidth;
  const height = Math.min(asset.atlas ? image.naturalHeight / 9 : image.naturalHeight, width * 2 / 3);
  context.drawImage(image, 0, 0, width, height, 0, 0, 36, 24);
  return canvas.toDataURL('image/png');
}
