<script lang="ts">
  import { t, i18n } from "./i18n.svelte";
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, baseName, type HistoryEntry } from "./types";

  let { onclose, onlog }: { onclose: () => void; onlog: (id: string, title: string) => void } = $props();

  let list = $state<HistoryEntry[]>([]);
  let loaded = $state(false);
  let q = $state("");
  let only = $state<"all" | "ok" | "error">("all");

  onMount(async () => {
    list = await api.loadHistory();
    loaded = true;
  });

  const shown = $derived(
    list.filter(
      (h) =>
        (only === "all" || (only === "ok" ? h.status === "ok" : h.status !== "ok")) &&
        (!q.trim() || (h.source + " " + h.output).toLowerCase().includes(q.trim().toLowerCase())),
    ),
  );
  const okCount = $derived(list.filter((h) => h.status === "ok").length);

  async function clearAll() {
    const yes = await ask(t("Tüm geçmiş ve iş kayıtları silinsin mi? İşlenmiş dosyalara dokunulmaz."), {
      title: t("Geçmişi temizle"),
      kind: "warning",
      okLabel: t("Sil"),
      cancelLabel: t("Vazgeç"),
    });
    if (!yes) return;
    await api.clearHistory();
    list = [];
  }

  const fmtTime = (ms: number) =>
    new Date(ms).toLocaleString(i18n.lang === "en" ? "en-GB" : "tr-TR", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
  const fmtSec = (s: number) => (s >= 60 ? t("{d}dk {s}sn", { d: Math.floor(s / 60), s: Math.round(s % 60) }) : t("{s}sn", { s: s.toFixed(1) }));
  const MODE: Record<string, string> = { local: "Yerel", target: "Hedef", inplace: "Yerinde" };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <div class="modal" style="width:min(920px, calc(100vw - 32px)); height:min(640px, calc(100vh - 48px))" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-modal="true" aria-label={t("Geçmiş")}>
    <header>
      <h2>{t("Geçmiş")}</h2>
      <button class="icon" onclick={onclose} aria-label={t("Kapat")}>✕</button>
    </header>

    <div class="tools">
      <input bind:value={q} placeholder={t("Dosya adında ara…")} />
      <div class="seg">
        <button class:on={only === "all"} onclick={() => (only = "all")}>{t("Hepsi")} ({list.length})</button>
        <button class:on={only === "ok"} onclick={() => (only = "ok")}>{t("Başarılı")} ({okCount})</button>
        <button class:on={only === "error"} onclick={() => (only = "error")}>{t("Hatalı")} ({list.length - okCount})</button>
      </div>
    </div>

    <div class="list">
      {#each shown as h (h.id)}
        <div class="row">
          <span class="chip {h.status === 'ok' ? 'ok' : h.status === 'error' ? 'err' : ''}">
            {h.status === "ok" ? t("Tamam") : h.status === "error" ? t("Hata") : t("İptal")}
          </span>
          <div class="main">
            <div class="n" title={h.source}>{baseName(h.source)}</div>
            <div class="s">
              {fmtTime(h.time)} · {fmtSec(h.seconds)} · {t(MODE[h.mode] ?? h.mode)}{h.encoded ? ` · ${t("{n} parça kodlandı", { n: h.encoded })}` : ""}
            </div>
            {#if h.status === "ok" && h.output}
              <div class="s mono" title={h.output}>→ {h.output}</div>
            {:else if h.error}
              <div class="s err">{h.error}</div>
            {/if}
          </div>
          <div class="acts">
            {#if h.status === "ok" && h.output}
              <button class="sm ghost" onclick={() => revealItemInDir(h.output)}>{t("Klasörde göster")}</button>
            {/if}
            <button class="sm ghost" onclick={() => onlog(h.id, baseName(h.source))}>{t("Log")}</button>
          </div>
        </div>
      {:else}
        <div class="empty muted">{loaded ? (list.length ? t("Eşleşen kayıt yok") : t("Henüz işlenmiş dosya yok")) : t("Yükleniyor…")}</div>
      {/each}
    </div>

    <footer>
      <button class="sm danger" onclick={clearAll} disabled={!list.length}>{t("Geçmişi temizle")}</button>
      <button class="sm ghost" onclick={onclose}>{t("Kapat")}</button>
    </footer>
  </div>
</div>

<style>
  .tools {
    display: flex;
    gap: 10px;
    padding: 12px 18px 6px;
    align-items: center;
  }
  .tools input {
    flex: 1;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 12px 12px;
  }
  .row {
    display: grid;
    grid-template-columns: 64px 1fr auto;
    gap: 12px;
    align-items: center;
    padding: 9px 8px;
    border-bottom: 1px solid var(--line-soft);
  }
  .row .chip {
    justify-self: start;
  }
  .main {
    min-width: 0;
  }
  .n {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
  }
  .s {
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: text;
  }
  .s.err {
    color: var(--err);
    white-space: normal;
  }
  .acts {
    display: flex;
    gap: 4px;
  }
  .empty {
    padding: 40px;
    text-align: center;
  }
</style>
