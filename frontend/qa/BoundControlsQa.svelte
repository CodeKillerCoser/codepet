<script lang="ts">
  import { Button } from "../lib/ui/button/index";
  import { Input } from "../lib/ui/input/index";
  import { Textarea } from "../lib/ui/textarea/index";
  export let initializeWithNull = false;
  let visible = true;
  let button: HTMLButtonElement | null | undefined = initializeWithNull ? null : undefined;
  let input: HTMLInputElement | null | undefined = initializeWithNull ? null : undefined;
  let textarea: HTMLTextAreaElement | null | undefined = initializeWithNull ? null : undefined;
</script>

<button type="button" onclick={() => visible = !visible}>切换挂载</button>
{#if visible}
  <Button bind:ref={button}>目标按钮</Button>
  <Input bind:ref={input} aria-label="目标输入" />
  <Textarea bind:ref={textarea} aria-label="目标文本域" />
{/if}
<output>{button?.tagName ?? "none"}/{input?.tagName ?? "none"}/{textarea?.tagName ?? "none"}</output>
