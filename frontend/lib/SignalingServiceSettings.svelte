<script lang="ts">
  import { Button } from "./ui/button/index";
  import { Input } from "./ui/input/index";
  export let value: string | null = null;
  export let onSave: (address: string) => Promise<string | null>;
  let address = value ?? "";
  let busy = false, error = "", saved = false;
  async function save() {
    if (busy) return;
    busy = true; error = ""; saved = false;
    try {
      address = (await onSave(address)) ?? "";
      saved = true;
    } catch (cause) {
      error = String(cause);
    } finally { busy = false; }
  }
</script>

<section class="settings-section" aria-labelledby="signaling-service-heading">
  <h3 id="signaling-service-heading">信令服务</h3>
  <div class="settings-card">
    <form on:submit|preventDefault={save}>
      <label for="signaling-service-address">服务器地址</label>
      <div class="address-row">
        <Input id="signaling-service-address" bind:value={address} oninput={() => saved = false}
          placeholder="IP 或域名" autocomplete="off" spellcheck={false} disabled={busy}
          aria-describedby="signaling-service-help" aria-invalid={!!error} />
        <Button type="submit" disabled={busy}>{busy ? "登记中…" : "保存并登记"}</Button>
      </div>
      <p id="signaling-service-help">填写 IP 或域名即可自动登记，默认 HTTPS、8443 端口；也可填写完整 HTTPS 地址。留空沿用原有服务配置。更换服务后，请让手机重新连接或配对。</p>
      {#if saved}<p role="status">{address ? "设备已登记，可开始配对。" : "已恢复原有服务配置。"}</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </form>
  </div>
</section>

<style>
  section{min-width:0;color:var(--app-text);font-family:var(--font-family-ui)}
  form{padding:18px 0}
  label{display:block;font-size:var(--font-size-label);font-weight:var(--font-weight-medium);margin-bottom:10px}
  .address-row{display:flex;align-items:center;gap:12px}
  .address-row :global(input){flex:1;min-width:0;background:var(--app-surface);color:var(--app-text)}
  .address-row :global(input),.address-row :global(button){padding:7px 12px;border:1px solid var(--app-border);border-radius:6px;font:inherit;font-size:var(--font-size-label)}
  .address-row :global(button){flex-shrink:0;background:var(--app-surface-soft);color:var(--app-text)}
  p{font-size:var(--font-size-caption);line-height:var(--line-height-body);color:var(--app-muted);overflow-wrap:anywhere;margin:10px 0 0}
  .error{color:var(--color-main-danger-text)}
  .address-row :global(:is(input,button):focus-visible){outline:2px solid var(--color-focus-ring);outline-offset:2px}
</style>
