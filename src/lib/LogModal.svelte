<script lang="ts">
  import { t } from "./i18n.svelte";
  import { onMount } from "svelte";
  let {
    title,
    load,
    hint = "",
    onclose,
  }: { title: string; load: () => Promise<string>; hint?: string; onclose: () => void } = $props();

  let text = $state("");
  let error = $state("");
  let copied = $state(false);

  onMount(async () => {
    try {
      text = await load();
    } catch (e) {
      error = String(e);
    }
  });

  async function copy() {
    await navigator.clipboard.writeText(text);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop top" onclick={onclose}>
  <div class="modal" style="width:min(900px, calc(100vw - 32px)); height:min(640px, calc(100vh - 48px))" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-modal="true" aria-label={title}>
    <header>
      <h2>{title}</h2>
      <button class="icon" onclick={onclose} aria-label={t("Kapat")}>✕</button>
    </header>
    {#if error}
      <div class="empty muted">{error}</div>
    {:else}
      <pre>{text || t("Yükleniyor…")}</pre>
    {/if}
    <footer>
      <span class="hint">{hint}</span>
      <div style="display:flex;gap:8px">
        <button class="sm" onclick={copy} disabled={!text}>{copied ? t("Kopyalandı") : t("Kopyala")}</button>
        <button class="sm ghost" onclick={onclose}>{t("Kapat")}</button>
      </div>
    </footer>
  </div>
</div>

<style>
  .top {
    z-index: 40;
  }
  pre {
    flex: 1;
    margin: 0;
    overflow: auto;
    padding: 14px 18px;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
    background: var(--bg);
  }
  .empty {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 24px;
  }
</style>
