import { invoke } from "@tauri-apps/api/core";
import orangeCat from "../../assets/pets/orange-cat.webp";
import panda from "../../assets/pets/panda.webp";
import type { PetKind } from "../types";

export const builtInPets = [
  { id: "default", name: "橘猫", url: orangeCat },
  { id: "builtin:panda", name: "熊猫", url: panda },
] as const;

export function builtInPet(id: string) {
  return builtInPets.find(pet => pet.id === id) ?? builtInPets[0];
}

export async function loadPetAsset(id: string, kind: PetKind | null | undefined, path?: string | null) {
  if (!path || kind === "builtin" || !kind) {
    return { url: builtInPet(id).url, atlas: true };
  }
  // A data URL also allows alpha hit-testing in the WebView without a tainted canvas.
  const url = await invoke<string>("pet_asset_data_url", { path });
  return { url, atlas: kind === "codex-atlas" };
}
