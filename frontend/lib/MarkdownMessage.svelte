<script lang="ts">
  import { isTauri } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { renderMessageMarkdown } from "./markdown";

  export let message: string;
  let linkError = "";
  $: html = renderMessageMarkdown(message);

  async function openLink(event: MouseEvent) {
    const anchor = (event.target as Element).closest<HTMLAnchorElement>("a[href]");
    if (!anchor) return;
    event.preventDefault();
    event.stopPropagation();
    const url = anchor.getAttribute("href") ?? "";
    if (!/^(https?:\/\/|mailto:)/i.test(url)) return;
    linkError = "";
    try {
      if (isTauri()) await openUrl(url);
      else window.open(url, "_blank", "noopener,noreferrer");
    } catch {
      linkError = "无法打开链接，请复制链接地址后打开";
    }
  }
</script>

<!-- The scroll region must be keyboard reachable. Links retain native keyboard activation. -->
<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions a11y_no_noninteractive_tabindex -->
<div class="status-message markdown-message" on:click={openLink} tabindex="0" role="region" aria-label="任务消息">
  {@html html}
</div>
{#if linkError}<span class="markdown-link-error" role="status">{linkError}</span>{/if}
