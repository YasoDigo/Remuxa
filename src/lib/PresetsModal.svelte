<script lang="ts">
  import { t } from "./i18n.svelte";
  import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { api, uid, type Rules, type Settings } from "./types";
  import { BUILTIN, blankRules, NONE_ID } from "./presets";

  let {
    settings = $bindable(),
    onclose,
    onnotice,
  }: { settings: Settings; onclose: () => void; onnotice: (m: string) => void } = $props();

  let user: Rules[] = $state(JSON.parse(JSON.stringify(settings.presets ?? [])));
  let selId = $state(BUILTIN[1].id);
  const all = $derived([...BUILTIN, ...user]);
  const sel = $derived(all.find((p) => p.id === selId) ?? BUILTIN[0]);
  const ro = $derived(!!sel.builtin);

  function duplicate() {
    const c: Rules = { ...JSON.parse(JSON.stringify(sel)), id: "u-" + uid(), name: (sel.builtin ? t(sel.name) : sel.name) + " (" + t("kopya") + ")", desc: sel.builtin ? t(sel.desc) : sel.desc, builtin: undefined };
    delete c.builtin;
    user.push(c);
    selId = c.id;
  }
  function create() {
    const c = blankRules();
    user.push(c);
    selId = c.id;
  }
  function remove() {
    user = user.filter((p) => p.id !== selId);
    selId = BUILTIN[1].id;
  }

  async function persist() {
    settings.presets = $state.snapshot(user) as Rules[];
    await api.saveSettings($state.snapshot(settings) as Settings);
  }
  async function saveClose() {
    await persist();
    onclose();
  }

  async function exportAll() {
    if (!user.length) return onnotice(t("Dışa aktarılacak kendi hazır ayarın yok"));
    const path = await saveDialog({ defaultPath: "remuxa-hazir-ayarlar.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    await api.writeText(path, JSON.stringify({ remuxaPresets: 1, presets: $state.snapshot(user) }, null, 2));
    onnotice(t("{n} hazır ayar dışa aktarıldı", { n: user.length }));
  }
  async function importFile() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    try {
      const data = JSON.parse(await api.readText(path));
      const list: Rules[] = Array.isArray(data) ? data : data.presets;
      if (!Array.isArray(list)) throw new Error(t("biçim tanınmadı"));
      const base = blankRules();
      for (const p of list) user.push({ ...base, ...p, id: "u-" + uid(), builtin: undefined });
      onnotice(t("{n} hazır ayar içe aktarıldı", { n: list.length }));
    } catch (e) {
      onnotice(t("İçe aktarılamadı:") + " " + (e instanceof Error ? e.message : String(e)));
    }
  }

  type Opt<T> = [T, string][];
  const AUDIO_LANGS: Opt<Rules["audioLangs"]> = [["all", "Hepsi"], ["pref", "Tercih edilenler"]];
  const SUBS: Opt<Rules["subLangs"]> = [["all", "Hepsi"], ["pref", "Tercih edilenler"], ["none", "Hiçbiri"]];
  const STEREO: Opt<Rules["stereo"]> = [["none", "Dokunma"], ["add", "Yanına stereo AAC ekle"], ["replace", "Stereo AAC ile değiştir"]];
  const DEFAULTS: Opt<Rules["defaults"]> = [["keep", "Olduğu gibi"], ["pref", "Dil önceliğine göre"]];
  const VCODEC: Opt<NonNullable<Rules["videoCodec"]>> = [["keep", "Olduğu gibi"], ["h264", "H.264"], ["hevc", "HEVC"], ["av1", "AV1"]];
  const CONTAINER: Opt<Rules["container"]> = [["keep", "Kaynakla aynı"], ["mkv", "MKV"], ["mp4", "MP4"]];
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

{#snippet seg<T extends string>(opts: [T, string][], value: T, set: (v: T) => void)}
  <div class="seg">
    {#each opts as [v, n]}
      <button class:on={value === v} onclick={() => set(v)} disabled={ro}>{t(n)}</button>
    {/each}
  </div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}>
  <div class="modal" style="width:min(860px, calc(100vw - 32px)); height:min(620px, calc(100vh - 48px))" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-modal="true" aria-label={t("Hazır ayarlar")}>
    <header>
      <h2>{t("Hazır ayarlar")}</h2>
      <button class="icon" onclick={onclose} aria-label={t("Kapat")}>✕</button>
    </header>

    <div class="cols">
      <aside>
        <div class="grp">{t("Yerleşik")}</div>
        {#each BUILTIN.filter((p) => p.id !== NONE_ID) as p (p.id)}
          <button class="item" class:on={selId === p.id} onclick={() => (selId = p.id)}>{t(p.name)}</button>
        {/each}
        <div class="grp">{t("Benim")}</div>
        {#each user as p (p.id)}
          <button class="item" class:on={selId === p.id} onclick={() => (selId = p.id)}>{p.name || t("(adsız)")}</button>
        {:else}
          <div class="hint empty">{t("Henüz yok. Yerleşik birinin kopyasını oluşturup değiştirebilirsin.")}</div>
        {/each}
        <button class="sm new" onclick={create}>{t("+ Yeni")}</button>
      </aside>

      <div class="form">
        {#if ro}
          <div class="ro"><span class="chip">{t("Yerleşik · değiştirilemez")}</span><button class="sm" onclick={duplicate}>{t("Kopyasını oluştur")}</button></div>
          <h3 class="pname">{t(sel.name)}</h3>
          <p class="hint">{t(sel.desc)}</p>
        {:else}
          <label class="field"><span>{t("Ad")}</span><input bind:value={sel.name} /></label>
          <label class="field"><span>{t("Açıklama")}</span><input bind:value={sel.desc} placeholder={t("Ne işe yaradığı (isteğe bağlı)")} /></label>
        {/if}

        <h3 class="sec">{t("Ses")}</h3>
        <div class="line"><span>{t("Diller")}</span>{@render seg(AUDIO_LANGS, sel.audioLangs, (v) => (sel.audioLangs = v))}</div>
        <div class="line"><span>{t("Uyumsuz ses")} <span class="hint">{t("(AC3, DTS, TrueHD, 5.1…)")}</span></span>{@render seg(STEREO, sel.stereo, (v) => (sel.stereo = v))}</div>
        {#if sel.stereo !== "none"}
          <div class="line">
            <span>{t("Stereo bit hızı")}</span>
            <select bind:value={sel.stereoBitrate} disabled={ro}>
              {#each [128, 160, 192, 224, 256] as b}<option value={b}>{b} kbps</option>{/each}
            </select>
          </div>
        {/if}
        {#if sel.stereo !== "none"}
          <label class="chk"><input type="checkbox" bind:checked={sel.dialog} disabled={ro} /> {t("Stereo karışımda diyaloğu öne çıkar")}</label>
          <label class="chk"><input type="checkbox" bind:checked={sel.normalize} disabled={ro} /> {t("Ses seviyesini eşitle (EBU R128)")}</label>
        {/if}
        <label class="chk"><input type="checkbox" bind:checked={sel.dropDuplicates} disabled={ro} /> {t("Birebir kopya ses parçalarını çıkar")}</label>
        <label class="chk"><input type="checkbox" bind:checked={sel.dropCommentary} disabled={ro} /> {t("Yorum (commentary) parçalarını çıkar")}</label>

        <h3 class="sec">{t("Altyazı ve varsayılanlar")}</h3>
        <div class="line"><span>{t("Altyazılar")}</span>{@render seg(SUBS, sel.subLangs, (v) => (sel.subLangs = v))}</div>
        <div class="line"><span>{t("Varsayılan parçalar")}</span>{@render seg(DEFAULTS, sel.defaults, (v) => (sel.defaults = v))}</div>
        <label class="chk"><input type="checkbox" bind:checked={sel.sidecars} disabled={ro} /> {t("Videonun yanındaki altyazı dosyalarını ekle")} <span class="hint">{t("(Film.tr.srt gibi)")}</span></label>
        <label class="chk"><input type="checkbox" bind:checked={sel.fixTr} disabled={ro} /> {t("Altyazılarda bozuk Türkçe karakterleri düzelt")} <span class="hint">{t("(ý þ ð → ı ş ğ)")}</span></label>

        <h3 class="sec">{t("Video")}</h3>
        <div class="line"><span>{t("Video codec'i")}</span>{@render seg(VCODEC, sel.videoCodec ?? "keep", (v) => (sel.videoCodec = v))}</div>
        {#if (sel.videoCodec ?? "keep") !== "keep"}
          <div class="line">
            <span>{t("En fazla çözünürlük")}</span>
            <select bind:value={sel.videoMaxHeight} disabled={ro}>
              <option value={0}>{t("Aynı")}</option>
              {#each [2160, 1440, 1080, 720, 480] as h}<option value={h}>{h}p</option>{/each}
            </select>
          </div>
          <p class="hint">{t("Zaten bu codec'te olan ve küçültme gerekmeyen videolara dokunulmaz. H.264'e çevrilen HDR videolar SDR yapılır.")}</p>
        {/if}

        <h3 class="sec">{t("Çıktı")}</h3>
        <div class="line"><span>{t("Kapsayıcı")}</span>{@render seg(CONTAINER, sel.container, (v) => (sel.container = v))}</div>
        <label class="chk"><input type="checkbox" bind:checked={sel.keepChapters} disabled={ro} /> {t("Bölüm işaretlerini tut")}</label>
        <label class="chk"><input type="checkbox" bind:checked={sel.keepAttachments} disabled={ro} /> {t("Ekleri tut (fontlar, kapak)")}</label>
        <label class="chk"><input type="checkbox" bind:checked={sel.clearTitle} disabled={ro} /> {t("Dosya başlığını temizle")}</label>

        <p class="hint foot">{t("\"Tercih edilenler\" ve \"dil önceliği\", Ayarlar'daki dil sırasını kullanır:")} <b>{settings.prefLangs}</b></p>
      </div>
    </div>

    <footer>
      <div style="display:flex;gap:6px">
        <button class="sm ghost" onclick={importFile}>{t("İçe aktar")}</button>
        <button class="sm ghost" onclick={exportAll}>{t("Dışa aktar")}</button>
        {#if !ro}<button class="sm danger" onclick={remove}>{t("Sil")}</button>{/if}
      </div>
      <div style="display:flex;gap:8px">
        <button class="ghost" onclick={onclose}>{t("Vazgeç")}</button>
        <button class="primary" onclick={saveClose}>{t("Kaydet")}</button>
      </div>
    </footer>
  </div>
</div>

<style>
  .cols {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 230px 1fr;
  }
  aside {
    border-right: 1px solid var(--line-soft);
    padding: 10px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .grp {
    font-size: 11px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.07em;
    padding: 10px 8px 4px;
  }
  .item {
    text-align: left;
    background: transparent;
    border-color: transparent;
    padding: 7px 10px;
  }
  .item.on {
    background: var(--accent-bg);
    color: var(--accent);
    font-weight: 600;
  }
  .empty {
    padding: 4px 8px 8px;
  }
  .new {
    margin: 8px 6px 0;
    align-self: flex-start;
  }
  .form {
    overflow-y: auto;
    padding: 14px 20px 16px;
  }
  .ro {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .pname {
    margin: 12px 0 4px;
    font-size: 16px;
  }
  .form :global(h3.sec) {
    margin-top: 18px;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 9px;
    font-size: 13px;
  }
  .chk {
    display: flex;
    gap: 9px;
    align-items: center;
    font-size: 13px;
    margin: 7px 0;
  }
  .foot {
    margin-top: 18px;
  }
</style>
