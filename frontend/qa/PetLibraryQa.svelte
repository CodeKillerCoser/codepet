<script lang="ts">
  import PetLibraryPanel from "../lib/pet/PetLibraryPanel.svelte";
  import type { AppSettings } from "../lib/types";
  export let initial: AppSettings;
  let settings = initial;
  let action = "";
  function select(id: string) {
    const pet = settings.petLibrary.pets.find(pet => pet.id === id)!;
    settings = { ...settings, pet: { ...settings.pet, selectedPetId: id, kind: pet.kind, imagePath: pet.imagePath } };
  }
</script>
<main class="main-theme light" style="padding:16px; height:100vh; overflow:auto;">
  <div class="settings-workspace pet-creation">
  <PetLibraryPanel {settings} onSelect={select} onRemove={(_, id) => action = `remove:${id}`} onImportImage={(cutout) => action = cutout ? "cutout" : "image"} onImportPackage={() => action = "package"} onScale={(scale) => settings = { ...settings, pet: { ...settings.pet, scale } }} />
  </div>
  <output>{action}</output>
</main>
