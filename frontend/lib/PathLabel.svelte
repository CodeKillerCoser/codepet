<script lang="ts">
  export let value: string;
  $: boundary = Math.max(value.lastIndexOf('/'), value.lastIndexOf('\\')) + 1;
  $: prefix = boundary ? value.slice(0, boundary) : '';
  $: tail = boundary ? value.slice(boundary) : value;
</script>
<span class="path-label" title={value} aria-label={value}><span class="prefix">{prefix}</span><span class="tail">{tail}</span></span>
<style>
  .path-label { display: flex; justify-content: flex-end; min-width: 0; max-width: 100%; white-space: nowrap; }
  .prefix { flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .tail { flex: 0 0 auto; max-width: 60%; overflow: hidden; text-overflow: ellipsis; }
  .prefix:empty { display: none; }
  .prefix:empty + .tail { max-width: 100%; }
</style>
