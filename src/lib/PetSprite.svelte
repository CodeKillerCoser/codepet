<script lang="ts">
  import { cssColorTokens, spriteAccentColorForStatus } from "./theme";
  import type { PixelPetSprite, TaskStatus } from "./types";

  export let sprite: PixelPetSprite;
  export let status: TaskStatus = "idle";
  export let scale = 3;

  const matrix = [
    "000000001111111100000000",
    "000000111111111111000000",
    "000011111111111111110000",
    "000111111111111111111000",
    "001111111111111111111100",
    "001111110000001111111100",
    "001111006666660011111100",
    "011100666666666600111110",
    "011006666666666666001110",
    "010066663666366666600110",
    "000666666666666666660000",
    "000666664666466666660000",
    "000066666444666666600000",
    "000006666666666666000000",
    "000000666666666600000000",
    "000001555555555510000000",
    "000015555555555551000000",
    "000155555555555555100000",
    "001555555555555555510000",
    "011555222555522255511000",
    "011552222255222225511000",
    "001552222222222225510000",
    "000155522222222555100000",
    "000011555555555511000000",
    "000000157700775100000000",
    "000000177000077100000000",
    "000001177000077110000000",
    "000001110000001110000000",
    "000011100000000111000000",
    "000000000000000000000000",
  ];

  $: colors = {
    "0": cssColorTokens.transparent,
    "1": sprite.accent,
    "2": cssColorTokens.petSpriteSoftWhite,
    "3": sprite.eyes,
    "4": cssColorTokens.petSpriteWhite,
    "5": spriteAccentColorForStatus(status),
    "6": sprite.body,
    "7": cssColorTokens.petSpriteFoot,
  };
</script>

<div
  class:thinking={status === "thinking"}
  class:running={status === "running" || status === "waiting-approval"}
  class:alerting={status === "waiting-approval" || status === "failed"}
  class="pet-sprite"
  style={`--pixel:${scale}px`}
  aria-label="Code Pet"
  role="img"
>
  {#each matrix as row}
    {#each row.split("") as cell}
      <span style={`background:${colors[cell as keyof typeof colors]}`}></span>
    {/each}
  {/each}
</div>
