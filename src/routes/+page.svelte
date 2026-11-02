<script lang="ts">
  import { t, i18n, resolveLang } from "$lib/i18n.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open, ask } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import SettingsModal from "$lib/SettingsModal.svelte";
  import PresetsModal from "$lib/PresetsModal.svelte";
  import TrackEditor from "$lib/TrackEditor.svelte";
  import HistoryModal from "$lib/HistoryModal.svelte";
  import LogModal from "$lib/LogModal.svelte";
  import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
  import {
    api, fmtSize, fmtDur, STAGES, DEFAULT_SETTINGS, uid, isSubPath,
    type Item, type Settings, type ToolStatus, type InputFile, type Rules,
  } from "$lib/types";
  import {
    BUILTIN, NONE_ID, applyRules, identity, signature, hasWork, layoutKey, cloneEdit, finalName, stem,
    attachExternal, jobTracks,
  } from "$lib/presets";

  let settings = $state<Settings>({ ...DEFAULT_SETTINGS });
  let tools = $state<ToolStatus | null>(null);
  let items = $state<Item[]>([]);
  let activeId = $state<string | null>(null);
  let running = $state(false);
  let stopping = $state(false);
  let dragging = $state(false);
  let showSettings = $state(false);
  let showPresets = $state(false);
  let showHistory = $state(false);
  let logView = $state<{ id: string; title: string } | null>(null);
  let pauseAfter = $state(false);
  let restored = $state(false);
  let bulkPreset = $state("media-server");
  let toast = $state("");
  let toastTimer: ReturnType<typeof setTimeout>;

  const presets = $derived<Rules[]>([...BUILTIN, ...(settings.presets ?? [])]);
  const active = $derived(items.find((i) => i.id === activeId) ?? null);
  const activeIdx = $derived(items.findIndex((i) => i.id === activeId));
  const toolsOk = $derived(!!tools?.ffmpeg && !!tools?.ffprobe && !!tools?.mkvmerge);
  const checked = $derived(items.filter((i) => i.checked));
  const work = $derived(items.filter((i) => i.status === "idle" && hasWork(i, settings.nameTemplate)));
  const probing = $derived(items.filter((i) => i.status === "probing").length);
  const encodeCount = $derived(work.filter((i) => i.edit!.tracks.some((t) => t.convert)).length);
  const destOk = $derived(
    settings.outMode === "inplace" ||
      (settings.outMode === "local" && !!settings.localDir.trim()) ||
      (settings.outMode === "target" && settings.targets.some((t) => t.path === settings.outTarget)),
  );
  const canStart = $derived(!running && toolsOk && destOk && work.length > 0 && probing === 0);

  // Klasöre göre gruplanmış liste
  const groups = $derived.by(() => {
    const m = new Map<string, Item[]>();
    for (const it of items) {
      const p = it.rel.split("/");
      const g = p.slice(0, -1).join(" / ");
      if (!m.has(g)) m.set(g, []);
      m.get(g)!.push(it);
    }
    return [...m.entries()];
  });

  function say(msg: string) {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ""), 3600);
  }

  async function persist() {
    await api.saveSettings($state.snapshot(settings) as Settings);
  }

  onMount(() => {
    const unsubs: (() => void)[] = [];
    (async () => {
      settings = { ...DEFAULT_SETTINGS, ...(await api.loadSettings()) };
      i18n.lang = resolveLang(settings.lang);
      if (settings.checkUpdates) checkUpdate();
      tools = await api.checkTools($state.snapshot(settings) as Settings);
      await restoreQueue();
      unsubs.push(
        await listen<{ id: string; stage: string; percent: number }>("job-progress", (e) => {
          const it = items.find((i) => i.logId === e.payload.id);
          if (it) {
            it.stage = e.payload.stage;
            it.percent = e.payload.percent;
          }
        }),
      );
      unsubs.push(
        await getCurrentWebview().onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "enter" || p.type === "over") dragging = !running;
          else if (p.type === "leave") dragging = false;
          else if (p.type === "drop") {
            dragging = false;
            if (running) return;
            const subs = p.paths.filter(isSubPath);
            if (subs.length) addSubs(subs);
            const rest = p.paths.filter((x) => !isSubPath(x));
            if (rest.length) addPaths(rest);
          }
        }),
      );
    })();
    return () => unsubs.forEach((u) => u());
  });

  $effect(() => {
    i18n.lang = resolveLang(settings.lang);
    document.documentElement.lang = i18n.lang;
  });

  // ─── Güncelleme ve araç kurulumu ───
  const REPO = "YasoDigo/Remuxa";
  let update = $state<{ version: string; url: string } | null>(null);
  let installing = $state(false);

  async function checkUpdate() {
    try {
      const cur = await getVersion();
      const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`);
      if (!r.ok) return;
      const rel = await r.json();
      const latest = String(rel.tag_name ?? "").replace(/^v/, "");
      const num = (v: string) => v.split(".").map((x) => parseInt(x) || 0);
      const [a, b] = [num(latest), num(cur)];
      for (let i = 0; i < 3; i++) {
        if ((a[i] ?? 0) > (b[i] ?? 0)) return (update = { version: latest, url: rel.html_url });
        if ((a[i] ?? 0) < (b[i] ?? 0)) return;
      }
    } catch {
      // çevrimdışı ya da repo erişilemiyor: sessiz geç
    }
  }

  async function installTools() {
    if (!tools) return;
    const which = [];
    if (!tools.ffmpeg || !tools.ffprobe) which.push("ffmpeg");
    if (!tools.mkvmerge) which.push("mkvtoolnix");
    installing = true;
    say(t("Kuruluyor… Bu birkaç dakika sürebilir, Windows izin isteyebilir."));
    try {
      await api.installTools($state.snapshot(settings) as Settings, which);
      tools = await api.checkTools($state.snapshot(settings) as Settings);
      say(toolsOk ? t("Araçlar kuruldu") : t("Kuruldu ama bulunamadı; uygulamayı yeniden başlat"));
    } catch (e) {
      say(String(e));
    } finally {
      installing = false;
    }
  }

  // ─── Kalıcı kuyruk ───
  async function restoreQueue() {
    try {
      const raw = await api.loadQueue();
      if (raw) {
        const data = JSON.parse(raw) as { items: Item[]; activeId: string | null };
        const list = (data.items ?? []).map((it) => {
          // Yarıda kalan iş tekrar sıraya girer, incelenmemiş dosya yeniden incelenir
          if (it.status === "running") return { ...it, status: "idle" as const, percent: 0, stage: "", error: "" };
          if (it.status === "cancelled") return { ...it, status: "idle" as const, error: "" };
          return it;
        });
        items = list;
        activeId = list.some((i) => i.id === data.activeId) ? data.activeId : (list[0]?.id ?? null);
        const reprobe = list.filter((i) => i.status === "probing" || !i.info).map((i) => i.id);
        for (const it of items) if (reprobe.includes(it.id)) it.status = "probing";
        if (reprobe.length) probeAll(reprobe);
        if (list.length) say(t("Önceki oturumdan {n} dosya geri yüklendi", { n: list.length }));
      }
    } catch {
      // bozuk kuyruk dosyası: boş başla
    }
    restored = true;
  }

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    if (!restored) return;
    const text = JSON.stringify({ items: $state.snapshot(items), activeId });
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => api.saveQueue(text).catch(() => {}), running ? 3000 : 600);
  });

  async function notify(title: string, body: string) {
    if (!settings.notify || document.hasFocus()) return;
    try {
      let ok = await isPermissionGranted();
      if (!ok) ok = (await requestPermission()) === "granted";
      if (ok) sendNotification({ title, body, sound: "Default" });
    } catch {
      // bildirim gönderilemezse sessizce geç
    }
  }

  // ─── Kuyruğa ekleme ───
  async function addPaths(paths: string[]) {
    if (!paths.length) return;
    const files = await api.expandPaths(paths);
    const known = new Set(items.map((i) => i.path));
    const fresh = files.filter((f) => !known.has(f.path));
    if (!files.length) return say(t("Video dosyası bulunamadı"));
    if (!fresh.length) return say(t("Bu dosyalar zaten listede"));
    const added = fresh.map(makeItem);
    items.push(...added);
    activeId ??= added[0].id;
    say(t("{n} dosya eklendi", { n: fresh.length }));
    await probeAll(added.map((i) => i.id));
  }

  /** Sürüklenen altyazı dosyaları açık dosyaya eklenir */
  async function addSubs(paths: string[]) {
    const it = active;
    if (!it?.info || !it.edit) return say(t("Altyazıyı eklemek için önce soldan bir video seç"));
    if (it.status === "done") return say(t("İşlenmiş dosyaya altyazı eklenemez; önce 'Tekrar işle'"));
    try {
      const list = await api.probeSubs($state.snapshot(settings) as Settings, it.path, paths);
      const n = attachExternal(it, list, true);
      if (it.status === "error" || it.status === "cancelled") it.status = "idle";
      say(n ? t("{n} altyazı \"{ad}\" dosyasına eklendi", { n, ad: stem(it.name) }) : t("Bu altyazı zaten listede"));
    } catch (e) {
      say(String(e));
    }
  }

  function makeItem(f: InputFile): Item {
    const parts = f.rel.split("/");
    return {
      id: uid(), path: f.path, rel: f.rel, name: parts[parts.length - 1], size: f.size,
      info: null, edit: null, base: "", presetId: NONE_ID, status: "probing", stage: "", percent: 0,
      error: "", output: "", checked: false,
    };
  }

  async function probeAll(ids: string[]) {
    let next = 0;
    const worker = async () => {
      while (next < ids.length) {
        const id = ids[next++];
        const it = items.find((i) => i.id === id);
        if (!it) continue;
        try {
          it.info = await api.probe($state.snapshot(settings) as Settings, it.path);
          it.edit = identity(it.info);
          it.base = signature(it.edit);
          it.presetId = NONE_ID;
          it.status = "idle";
        } catch (e) {
          it.status = "error";
          it.error = String(e);
        }
      }
    };
    await Promise.all([worker(), worker(), worker()]);
  }

  async function pickFiles() {
    const r = await open({ multiple: true, filters: [{ name: "Video", extensions: ["mkv", "mp4", "m4v", "avi", "ts", "m2ts", "mov", "webm"] }] });
    if (r) addPaths(Array.isArray(r) ? r : [r]);
  }
  async function pickFolder() {
    const r = await open({ directory: true, multiple: true });
    if (r) addPaths(Array.isArray(r) ? r : [r]);
  }

  // ─── Hazır ayarlar ───
  function applyPreset(it: Item, id: string) {
    if (!it.info || it.status === "running" || it.status === "done") return false;
    const rules = presets.find((p) => p.id === id);
    if (!rules) return false;
    it.edit = applyRules(it.info, rules, settings.prefLangs);
    it.base = signature(it.edit);
    it.presetId = id;
    it.status = "idle";
    it.error = "";
    return true;
  }

  function bulkApply() {
    const n = checked.filter((it) => applyPreset(it, bulkPreset)).length;
    say(t("\"{ad}\" {n} dosyaya uygulandı", { ad: t(presets.find((p) => p.id === bulkPreset)?.name ?? ""), n }));
  }

  function bulkCopyLayout() {
    if (!active?.info || !active.edit) return;
    const key = layoutKey(active.info);
    let ok = 0;
    let skip = 0;
    for (const it of checked) {
      if (it === active || !it.info || it.status === "running" || it.status === "done") continue;
      if (layoutKey(it.info) !== key) {
        skip++;
        continue;
      }
      it.edit = cloneEdit($state.snapshot(active.edit) as Item["edit"] & object);
      it.base = active.base;
      it.presetId = active.presetId;
      it.status = "idle";
      it.error = "";
      ok++;
    }
    say(t("Düzen {n} dosyaya kopyalandı", { n: ok }) + (skip ? " · " + t("{n} dosya farklı yapıda olduğu için atlandı", { n: skip }) : ""));
  }

  function removeChecked() {
    const ids = new Set(checked.filter((i) => i.status !== "running").map((i) => i.id));
    items = items.filter((i) => !ids.has(i.id));
    if (activeId && ids.has(activeId)) activeId = items[0]?.id ?? null;
  }

  function clearFinished() {
    items = items.filter((i) => !(i.status === "done" || (i.status === "idle" && !hasWork(i, settings.nameTemplate))));
    if (!items.some((i) => i.id === activeId)) activeId = items[0]?.id ?? null;
  }

  async function clearAll() {
    if (running || !items.length) return;
    const yes = await ask(t("Listedeki {n} dosyanın hepsi kaldırılsın mı? Dosyaların kendisine dokunulmaz.", { n: items.length }), {
      title: t("Listeyi temizle"),
      kind: "warning",
      okLabel: t("Temizle"),
      cancelLabel: t("Vazgeç"),
    });
    if (!yes) return;
    items = [];
    activeId = null;
  }

  function toggleAll(on: boolean) {
    for (const it of items) it.checked = on;
  }

  function requeue(it: Item) {
    it.status = "idle";
    it.error = "";
    it.percent = 0;
  }

  // ─── Çalıştırma ───
  async function start() {
    if (settings.outMode === "inplace") {
      const ok = await ask(
        t("{n} dosya kendi yerinde değiştirilecek (orijinalin üzerine yazılır). Çıkarılan parçalar geri gelmez. Devam edilsin mi?", { n: work.length }),
        { title: t("Yerinde dönüştürme"), kind: "warning", okLabel: t("Devam"), cancelLabel: t("Vazgeç") },
      );
      if (!ok) return;
    }
    running = true;
    stopping = false;
    pauseAfter = false;
    let ok = 0;
    let fail = 0;
    let paused = false;
    for (const it of [...work]) {
      if (stopping) break;
      if (pauseAfter) {
        paused = true;
        break;
      }
      if (it.status !== "idle") continue;
      it.status = "running";
      it.stage = "probe";
      it.percent = 0;
      it.error = "";
      const runId = `${it.id}-${Date.now().toString(36)}`;
      it.logId = runId;
      const e = $state.snapshot(it.edit!);
      try {
        const res = await api.convert($state.snapshot(settings) as Settings, {
          id: runId, path: it.path, rel: it.rel, mode: settings.outMode, targetDir: settings.outTarget,
          container: e.container, outName: finalName(it, settings.nameTemplate),
          ...jobTracks($state.snapshot(it.info!), e),
          keepChapters: e.keepChapters, keepAttachments: e.keepAttachments, clearTitle: e.clearTitle,
        });
        it.status = "done";
        it.output = res.output;
        it.percent = 100;
        ok++;
      } catch (err) {
        if (stopping) {
          it.status = "cancelled";
          it.error = t("İptal edildi");
        } else {
          it.status = "error";
          it.error = String(err);
          fail++;
        }
      }
    }
    running = false;
    const left = work.length;
    const summary = t("{n} başarılı", { n: ok }) + (fail ? ", " + t("{n} hata", { n: fail }) : "");
    if (stopping) say(t("Durduruldu"));
    else if (paused) say(t("Duraklatıldı · {ozet} · kalan {n} dosya için Başlat'a bas", { ozet: summary, n: left }));
    else {
      say(t("Bitti: {ozet}", { ozet: summary }));
      notify("Remuxa", t("İşlem bitti: {ozet}", { ozet: summary }));
    }
    stopping = false;
    pauseAfter = false;
  }

  async function stop() {
    stopping = true;
    await api.cancel();
  }

  // ─── Hedef seçimi ───
  function openLog(id: string, title: string) {
    logView = { id, title };
  }

  const destValue = $derived(settings.outMode === "target" ? "t:" + settings.outTarget : settings.outMode);
  async function setDest(v: string) {
    if (v.startsWith("t:")) {
      settings.outMode = "target";
      settings.outTarget = v.slice(2);
    } else {
      settings.outMode = v as Settings["outMode"];
    }
    await persist();
  }

  // ─── Liste etiketi ───
  function badge(it: Item): { text: string; cls: string } {
    switch (it.status) {
      case "probing":
        return { text: t("İnceleniyor…"), cls: "" };
      case "running":
        return { text: it.percent ? `%${Math.floor(it.percent)}` : t("Başlıyor"), cls: "accent" };
      case "done":
        return { text: t("Tamam"), cls: "ok" };
      case "error":
        return { text: t("Hata"), cls: "err" };
      case "cancelled":
        return { text: t("İptal"), cls: "" };
    }
    if (!hasWork(it, settings.nameTemplate)) return { text: t("Değişiklik yok"), cls: "" };
    const modified = it.edit && signature(it.edit) !== it.base;
    const p = presets.find((x) => x.id === it.presetId);
    return { text: modified || !p || p.id === NONE_ID ? t("Özel") : t(p.name), cls: "accent" };
  }

  function onKey(e: KeyboardEvent) {
    if ((e.target as HTMLElement).closest("input, select, textarea") || document.querySelector(".backdrop")) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const i = Math.max(0, Math.min(items.length - 1, activeIdx + (e.key === "ArrowDown" ? 1 : -1)));
      activeId = items[i]?.id ?? null;
      document.querySelector(`[data-item="${activeId}"]`)?.scrollIntoView({ block: "nearest" });
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<main>
  <header class="top">
    <div class="logo" aria-hidden="true"><svg viewBox="0 0 24 24"><rect x="3.5" y="7" width="6" height="1.7" rx=".85" /><rect x="3.5" y="11.15" width="4.6" height="1.7" rx=".85" opacity=".8" /><rect x="3.5" y="15.3" width="6" height="1.7" rx=".85" opacity=".6" /><path d="M11.5 6.2 19.5 12l-8 5.8z" /></svg></div>
    <div class="brand">
      <h1>{t("Remuxa")}</h1>
      <p>{t("Video parçalarını kayıpsız düzenle, gerekeni dönüştür")}</p>
    </div>
    <span class="sp"></span>
    {#if update}
      <button class="sm update" onclick={() => openUrl(update!.url)} title={t("İndirme sayfasını aç")}>{t("Yeni sürüm: v{v}", { v: update.version })}</button>
    {/if}
    <button class="sm ghost" onclick={pickFiles} disabled={running}>{t("+ Dosya")}</button>
    <button class="sm ghost" onclick={pickFolder} disabled={running}>{t("+ Klasör")}</button>
    <button class="sm ghost" onclick={clearFinished} disabled={running || !items.length}>{t("Bitenleri temizle")}</button>
    <button class="sm ghost" onclick={() => (showPresets = true)}>{t("Hazır ayarlar")}</button>
    <button class="sm ghost" onclick={() => (showHistory = true)}>{t("Geçmiş")}</button>
    <button class="icon" onclick={() => (showSettings = true)} aria-label={t("Ayarlar")} title={t("Ayarlar")}>
      <svg class="i" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.6 8.9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.4 9c.14.32.37.6.66.78.3.2.65.3 1 .3H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51.92Z" /></svg>
    </button>
  </header>

  {#if tools && !toolsOk}
    <div class="banner">
      <strong>{t("Eksik araç:")}</strong>
      {[!tools.ffmpeg && "ffmpeg", !tools.ffprobe && "ffprobe", !tools.mkvmerge && "mkvmerge"].filter(Boolean).join(", ")}.
      <button class="sm primary" onclick={installTools} disabled={installing}>{installing ? t("Kuruluyor…") : t("Otomatik kur")}</button>
      <span class="muted">{t("ya da terminalde:")}</span>
      {#if !tools.ffmpeg || !tools.ffprobe}<code>winget install Gyan.FFmpeg</code>{/if}
      {#if !tools.mkvmerge}<code>winget install MoritzBunkus.MKVToolNix</code>{/if}
      <span class="muted">{t("ya da Ayarlar'dan exe yolunu ver.")}</span>
    </div>
  {/if}

  {#if items.length === 0}
    <section class="drop">
      <div class="drop-icon" aria-hidden="true">
        <svg class="i" viewBox="0 0 24 24"><path d="M12 3v12m0 0-4.5-4.5M12 15l4.5-4.5" /><path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3" /></svg>
      </div>
      <h2>{t("Videoları ya da dizi klasörünü buraya bırak")}</h2>
      <p>{t("Parçaları incele, istediğini tut, çıkar, sırala ya da dönüştür. Klasör bırakırsan alt klasörler de taranır.")}</p>
      <div class="drop-actions">
        <button onclick={pickFiles}>{t("Dosya seç")}</button>
        <button onclick={pickFolder}>{t("Klasör seç")}</button>
      </div>
    </section>
  {:else}
    <div class="split">
      <aside class="left">
        <div class="lhead">
          <input type="checkbox" checked={checked.length === items.length} indeterminate={checked.length > 0 && checked.length < items.length} onchange={(e) => toggleAll((e.currentTarget as HTMLInputElement).checked)} title={t("Tümünü seç")} />
          <b>{t("{n} dosya", { n: items.length })}</b>
          {#if checked.length}<span class="muted">· {t("{n} seçili", { n: checked.length })}</span>{/if}
          {#if probing}<span class="chip">{t("{n} inceleniyor", { n: probing })}</span>{/if}
          <span class="sp"></span>
          <button class="link" onclick={clearAll} disabled={running} title={t("Tüm dosyaları listeden kaldır")}>{t("Listeyi temizle")}</button>
        </div>
        <div class="files">
          {#each groups as [g, list] (g)}
            {#if g}<div class="grp" title={g}>{g}</div>{/if}
            {#each list as it (it.id)}
              {@const b = badge(it)}
              <div class="f" class:sel={it.id === activeId} class:run={it.status === "running"} data-item={it.id} onclick={() => (activeId = it.id)} role="button" tabindex="-1" onkeydown={() => {}}>
                <input type="checkbox" bind:checked={it.checked} onclick={(e) => e.stopPropagation()} />
                <div class="fm">
                  <div class="n" title={it.path}>{stem(it.name)}</div>
                  <div class="s">
                    {fmtSize(it.size)}{#if it.info?.duration} · {fmtDur(it.info.duration)}{/if}
                    {#if it.status === "running"} · {t(STAGES[it.stage] ?? "")}{/if}
                  </div>
                  {#if it.status === "running"}
                    <div class="bar"><i style="width:{it.percent}%"></i></div>
                  {/if}
                </div>
                <span class="chip {b.cls}" title={b.text}>{b.text}</span>
              </div>
            {/each}
          {/each}
        </div>
        {#if checked.length}
          <div class="lfoot">
            <div class="muted">{t("Seçili {n} dosyaya:", { n: checked.length })}</div>
            <div class="r">
              <select bind:value={bulkPreset}>
                {#each presets as p (p.id)}<option value={p.id}>{t(p.name)}</option>{/each}
              </select>
              <button class="sm" onclick={bulkApply} disabled={running}>{t("Uygula")}</button>
            </div>
            <div class="r">
              <button class="sm" onclick={bulkCopyLayout} disabled={running || !active?.edit} title={t("Açık dosyadaki parça düzenini, aynı yapıdaki seçili dosyalara kopyalar")}>{t("Açık dosyanın düzenini kopyala")}</button>
              <button class="sm ghost" onclick={removeChecked} disabled={running}>{t("Listeden kaldır")}</button>
            </div>
          </div>
        {/if}
      </aside>

      <div class="right">
        {#if active?.info && active.edit}
          {#key active.id}
            <TrackEditor
              bind:item={items[activeIdx]}
              {presets}
              {settings}
              onpreset={(id) => applyPreset(items[activeIdx], id)}
              onnotice={say}
            >
              {#snippet actions()}
                {#if active.logId && (active.status === "done" || active.status === "error" || active.status === "cancelled")}
                  <button class="sm ghost" onclick={() => openLog(active.logId!, active.name)}>{t("Log")}</button>
                {/if}
                {#if active.status === "done"}<button class="sm" onclick={() => revealItemInDir(active.output)}>{t("Klasörde göster")}</button>{/if}
                {#if active.status === "done" || active.status === "error" || active.status === "cancelled"}
                  <button class="sm" onclick={() => requeue(items[activeIdx])} disabled={running}>{active.status === "done" ? t("Tekrar işle") : t("Tekrar dene")}</button>
                {/if}
              {/snippet}
            </TrackEditor>
          {/key}
        {:else if active?.status === "error"}
          <div class="empty err">{active.error}</div>
        {:else}
          <div class="empty muted">{active ? t("İnceleniyor…") : t("Soldan bir dosya seç")}</div>
        {/if}
      </div>
    </div>
  {/if}

  <footer class="bottom">
    <label class="dest">
      <span class="muted">{t("Hedef")}</span>
      <select value={destValue} onchange={(e) => setDest((e.currentTarget as HTMLSelectElement).value)} disabled={running}>
        <option value="local">{t("Yerel")}{settings.localDir ? ` — ${settings.localDir}` : ` (${t("klasör seçilmedi")})`}</option>
        {#each settings.targets as t (t.path)}
          <option value={"t:" + t.path}>{t.name} — {t.path}</option>
        {/each}
        <option value="inplace">{t("Yerinde (orijinalin üzerine)")}</option>
      </select>
    </label>
    <label class="tpl">
      <span class="muted">{t("Ad")}</span>
      <input class="mono" bind:value={settings.nameTemplate} onchange={persist} disabled={running} title={i18n.lang === "en" ? "{name} {series} {season} {episode} {folder}" : "{ad} {dizi} {sezon} {bolum} {klasor}"} />
    </label>
    <span class="sum">
      {#if running}
        {t("İşleniyor")}{work.length ? ` · ${t("{n} dosya sırada", { n: work.length })}` : ""}
      {:else if !destOk}
        <button class="link warn" onclick={() => (showSettings = true)}>{t("Hedef klasör ayarlanmadı")}</button>
      {:else if work.length}
        {t("{n} dosya sırada", { n: work.length })}{encodeCount ? ` · ${t("{n} tanesinde kodlama", { n: encodeCount })}` : ` · ${t("sadece remux")}`}
      {:else if items.length}
        {t("Yapılacak iş yok — dosya seçip hazır ayar uygula ya da parçaları düzenle")}
      {/if}
    </span>
    {#if running}
      <button class:on={pauseAfter} class="pause" onclick={() => (pauseAfter = !pauseAfter)} disabled={stopping} title={t("Şu anki dosya bitince dur; kalanlar sırada bekler")}>
        {pauseAfter ? t("Bu dosyadan sonra duracak") : t("Bitince dur")}
      </button>
      <button class="danger" onclick={stop} disabled={stopping}>{stopping ? t("Durduruluyor…") : t("Durdur")}</button>
    {:else}
      <button class="primary" onclick={start} disabled={!canStart}>{t("Başlat")}{work.length ? ` (${work.length})` : ""}</button>
    {/if}
  </footer>

  {#if dragging}
    <div class="drag-overlay"><div>{t("Bırak, listeye ekleyeyim")}</div></div>
  {/if}
  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
</main>

{#if showSettings}
  <SettingsModal bind:settings bind:tools onclose={() => (showSettings = false)} />
{/if}
{#if showPresets}
  <PresetsModal bind:settings onclose={() => (showPresets = false)} onnotice={say} />
{/if}
{#if showHistory}
  <HistoryModal onclose={() => (showHistory = false)} onlog={openLog} />
{/if}
{#if logView}
  {@const id = logView.id}
  <LogModal title={`Log · ${logView.title}`} load={() => api.readJobLog(id)} hint={t("Hata bildirirken bu metni paylaşabilirsin.")} onclose={() => (logView = null)} />
{/if}

<style>
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--line-soft);
  }
  .logo {
    width: 32px;
    height: 32px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    margin-right: 6px;
  }
  .logo svg {
    width: 16px;
    height: 16px;
    fill: white;
  }
  .brand h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 650;
  }
  .brand p {
    margin: 1px 0 0;
    color: var(--muted);
    font-size: 11.5px;
  }
  .sp {
    flex: 1;
  }
  .banner {
    margin: 10px 14px 0;
    background: var(--warn-bg);
    border: 1px solid rgb(251 191 36 / 0.35);
    border-radius: 10px;
    padding: 9px 13px;
    font-size: 13px;
    line-height: 1.8;
  }
  .banner code {
    background: var(--panel-2);
    border: 1px solid var(--line);
    padding: 1px 7px;
    border-radius: 6px;
    font-family: var(--mono);
    font-size: 12px;
    margin-right: 4px;
    user-select: text;
    white-space: nowrap;
  }

  .drop {
    flex: 1;
    margin: 14px;
    border: 1.5px dashed var(--line);
    border-radius: 16px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    text-align: center;
    padding: 24px;
  }
  .drop-icon {
    width: 56px;
    height: 56px;
    border-radius: 16px;
    background: var(--accent-bg);
    color: var(--accent);
    display: grid;
    place-items: center;
    margin-bottom: 8px;
  }
  .drop-icon svg {
    width: 26px;
    height: 26px;
  }
  .drop h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  .drop p {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
    max-width: 460px;
  }
  .drop-actions {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }

  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 300px 1fr;
  }
  .left {
    border-right: 1px solid var(--line-soft);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .lhead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    font-size: 12.5px;
  }
  .lhead .link {
    font-size: 12px;
    color: var(--muted);
  }
  .lhead .link:hover:not(:disabled) {
    color: var(--err);
  }
  .files {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .grp {
    font-size: 10.5px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 10px 8px 3px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .f {
    display: grid;
    grid-template-columns: 16px 1fr auto;
    gap: 9px;
    align-items: center;
    padding: 7px 8px;
    border-radius: 8px;
    border: 1px solid transparent;
    cursor: pointer;
  }
  .f:hover {
    background: var(--panel);
  }
  .f.sel {
    background: var(--accent-bg);
    border-color: rgb(139 124 246 / 0.35);
  }
  .fm {
    min-width: 0;
  }
  .f .n {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .f .s {
    font-size: 11px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .f .chip {
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    height: 3px;
    background: var(--line-soft);
    border-radius: 99px;
    margin-top: 5px;
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    transition: width 0.25s linear;
  }
  .lfoot {
    border-top: 1px solid var(--line-soft);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
  }
  .lfoot .r {
    display: flex;
    gap: 6px;
  }
  .lfoot select {
    flex: 1;
    padding: 4px 6px;
  }

  .right {
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .empty {
    margin: auto;
    padding: 24px;
    text-align: center;
    max-width: 520px;
    user-select: text;
  }
  .empty.err {
    color: var(--err);
  }

  .bottom {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 14px;
    border-top: 1px solid var(--line-soft);
  }
  .bottom label {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .dest select {
    max-width: 330px;
  }
  .tpl input {
    width: 170px;
  }
  .sum {
    flex: 1;
    color: var(--muted);
    font-size: 12.5px;
    min-width: 0;
  }
  .warn {
    color: var(--warn);
  }
  .update {
    border-color: var(--ok);
    color: var(--ok);
  }
  .pause.on {
    border-color: var(--warn);
    color: var(--warn);
  }

  .drag-overlay {
    position: fixed;
    inset: 10px;
    border: 2px dashed var(--accent);
    border-radius: 18px;
    background: rgb(15 17 23 / 0.82);
    backdrop-filter: blur(4px);
    display: grid;
    place-items: center;
    font-size: 18px;
    font-weight: 600;
    color: var(--accent);
    z-index: 30;
    pointer-events: none;
  }
  .toast {
    position: fixed;
    bottom: 66px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--panel-2);
    border: 1px solid var(--line);
    padding: 8px 16px;
    border-radius: 10px;
    font-size: 13px;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
    z-index: 25;
    max-width: 80vw;
    animation: fade 0.18s ease-out;
  }
</style>
