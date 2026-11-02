<script lang="ts">
  import { t } from "./i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, CHARSETS, type Settings, type ToolStatus } from "./types";

  let {
    settings = $bindable(),
    tools = $bindable(),
    onclose,
  }: { settings: Settings; tools: ToolStatus | null; onclose: () => void } = $props();

  let draft: Settings = $state(JSON.parse(JSON.stringify(settings)));
  let saving = $state(false);

  async function recheck() {
    tools = await api.checkTools($state.snapshot(draft));
  }

  async function pickDir(get: () => string, set: (v: string) => void) {
    const r = await open({ directory: true, defaultPath: get() || undefined });
    if (typeof r === "string") set(r);
  }

  async function pickExe(key: "ffmpegPath" | "ffprobePath" | "mkvmergePath") {
    const r = await open({ filters: [{ name: "Program", extensions: ["exe"] }] });
    if (typeof r === "string") {
      draft[key] = r;
      await recheck();
    }
  }

  function addTarget() {
    draft.targets.push({ name: draft.targets.length ? `${t("Hedef")} ${draft.targets.length + 1}` : "NAS", path: "" });
  }

  async function save() {
    saving = true;
    try {
      draft.targets = draft.targets
        .map((t) => ({ name: t.name.trim() || t.path.trim(), path: t.path.trim() }))
        .filter((t) => t.path);
      if (draft.outMode === "target" && !draft.targets.some((t) => t.path === draft.outTarget)) {
        draft.outMode = draft.targets.length ? "target" : "local";
        draft.outTarget = draft.targets[0]?.path ?? "";
      }
      const snap = $state.snapshot(draft) as Settings;
      await api.saveSettings(snap);
      settings = snap;
      await recheck();
      onclose();
    } finally {
      saving = false;
    }
  }

  const toolRows = [
    { key: "ffmpegPath", tool: "ffmpeg", label: "ffmpeg" },
    { key: "ffprobePath", tool: "ffprobe", label: "ffprobe" },
    { key: "mkvmergePath", tool: "mkvmerge", label: "mkvmerge" },
  ] as const;
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <div class="modal" style="width:min(680px, calc(100vw - 32px))" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-modal="true" aria-label={t("Ayarlar")}>
    <header>
      <h2>{t("Ayarlar")}</h2>
      <button class="icon" onclick={onclose} aria-label={t("Kapat")}>✕</button>
    </header>

    <div class="body">
      <section>
        <h3 class="sec">{t("Çıktı")}</h3>
        <label class="field">
          <span>{t("Yerel çıktı klasörü")}</span>
          <div class="row">
            <input bind:value={draft.localDir} placeholder={t("Örn: D:\\Videolar\\Dönüştürülenler")} />
            <button onclick={() => pickDir(() => draft.localDir, (v) => (draft.localDir = v))}>{t("Seç")}</button>
          </div>
        </label>

        <div class="field">
          <span>{t("Kayıtlı hedefler")} <span class="hint">{t("(NAS, harici disk… alttaki hedef seçicide görünür)")}</span></span>
          {#each draft.targets as tg, i}
            <div class="row">
              <input class="tname" bind:value={tg.name} placeholder={t("Ad")} />
              <input bind:value={tg.path} placeholder={t("Örn:") + " \\\\192.168.1.10\\Medya\\Diziler  " + t("veya") + "  Z:\\Diziler"} />
              <button onclick={() => pickDir(() => tg.path, (v) => (tg.path = v))}>{t("Seç")}</button>
              <button class="icon" onclick={() => draft.targets.splice(i, 1)} aria-label={t("Hedefi sil")} title={t("Sil")}>✕</button>
            </div>
          {/each}
          <div><button class="sm" onclick={addTarget}>{t("+ Hedef ekle")}</button></div>
          <span class="hint">{t("Ağ paylaşımına önce Gezgin'den bir kez girip oturum aç. Hedeflere yazılan dosyalar önce çalışma klasöründe hazırlanır, sonra kopyalanır.")}</span>
        </div>

        <label class="check">
          <input type="checkbox" bind:checked={draft.keepStructure} />
          <span>{t("Klasör yapısını koru")} <span class="hint">{t("(bıraktığın \"Dizi/Sezon 1\" klasörleri hedefte de oluşur)")}</span></span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.overwrite} />
          <span>{t("Hedefte aynı adda dosya varsa üzerine yaz")}</span>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={draft.notify} />
          <span>{t("İşler bitince Windows bildirimi gönder")} <span class="hint">{t("(uygulama arka plandayken)")}</span></span>
        </label>
      </section>

      <section>
        <h3 class="sec">{t("Ses, dil ve altyazı")}</h3>
        <div class="grid2">
          <label class="field">
            <span>{t("\"Türet\" için bit hızı")}</span>
            <select bind:value={draft.bitrate}>
              {#each [128, 160, 192, 224, 256] as b}
                <option value={b}>{b} kbps</option>
              {/each}
            </select>
          </label>
          <label class="field">
            <span>{t("Dil önceliği")}</span>
            <input bind:value={draft.prefLangs} placeholder={t("tur,jpn,eng")} />
          </label>
        </div>
        <span class="hint">{t("3 harfli dil kodları (tur, jpn, eng…). Hazır ayarlar varsayılan parçayı ve tutulacak dilleri buna göre seçer.")}</span>
        <label class="field" style="margin-top:12px">
          <span>{t("UTF-8 olmayan altyazıların kodlaması")}</span>
          <select bind:value={draft.subCharset}>
            {#each CHARSETS.filter(([c]) => c !== "utf-8") as [c, n] (c)}<option value={c}>{t(n)}</option>{/each}
          </select>
          <span class="hint">{t("Eski .srt dosyaları genelde bu kodlamadadır. Parça başına ayrıca değiştirilebilir.")}</span>
        </label>
      </section>

      <section>
        <h3 class="sec">{t("Uygulama")}</h3>
        <div class="grid2">
          <label class="field">
            <span>Dil / Language</span>
            <select bind:value={draft.lang}>
              <option value="">{t("Sistem dili")}</option>
              <option value="tr">Türkçe</option>
              <option value="en">English</option>
            </select>
          </label>
        </div>
        <label class="check">
          <input type="checkbox" bind:checked={draft.checkUpdates} />
          <span>{t("Açılışta yeni sürümü denetle")} <span class="hint">(GitHub)</span></span>
        </label>
      </section>

      <section>
        <h3 class="sec">{t("Araçlar")}</h3>
        {#each toolRows as r}
          <label class="field">
            <span>
              {r.label}
              {#if tools}
                {#if tools[r.tool]}<span class="chip ok">{t("bulundu")}</span>{:else}<span class="chip err">{t("yok")}</span>{/if}
              {/if}
            </span>
            <div class="row">
              <input bind:value={draft[r.key]} placeholder={tools?.[r.tool] ?? t("Otomatik (PATH, winget, Program Files)")} onchange={recheck} />
              <button onclick={() => pickExe(r.key)}>{t("Seç")}</button>
            </div>
          </label>
        {/each}
        <label class="field">
          <span>{t("Çalışma (geçici) klasörü")}</span>
          <div class="row">
            <input bind:value={draft.workDir} placeholder={t("Varsayılan: %TEMP%\\remuxa")} />
            <button onclick={() => pickDir(() => draft.workDir, (v) => (draft.workDir = v))}>{t("Seç")}</button>
          </div>
          <span class="hint">{t("Ses kodlama ve hedefe kopyalamadan önceki hazırlık burada yapılır. Hızlı bir SSD'de olsun.")}</span>
        </label>
      </section>
    </div>

    <footer>
      <span></span>
      <div style="display:flex;gap:8px">
        <button class="ghost" onclick={onclose}>{t("Vazgeç")}</button>
        <button class="primary" onclick={save} disabled={saving}>{t("Kaydet")}</button>
      </div>
    </footer>
  </div>
</div>

<style>
  .body {
    overflow-y: auto;
    padding: 4px 18px 12px;
  }
  section {
    padding: 14px 0 6px;
    border-bottom: 1px solid var(--line-soft);
  }
  section:last-child {
    border-bottom: 0;
  }
  .row {
    display: flex;
    gap: 6px;
    margin-bottom: 4px;
  }
  .row input {
    flex: 1;
  }
  .row input.tname {
    flex: 0 0 130px;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .check {
    display: flex;
    gap: 9px;
    align-items: flex-start;
    font-size: 13px;
    margin: 8px 0;
    cursor: pointer;
  }
  .check input {
    margin-top: 2px;
  }
</style>
