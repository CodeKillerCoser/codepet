<script lang="ts">
  export let label: string;
  export let description = "";
</script>
<div class="settings-row">
  <div class="settings-row-label"><span>{label}</span>{#if description}<p>{description}</p>{/if}</div>
  <div class="settings-row-value"><slot /></div>
</div>
