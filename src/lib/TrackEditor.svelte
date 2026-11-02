<script lang="ts" module>
  import type { EncCaps as Caps } from "./types";
  /** Kodlayıcı taraması pahalı: uygulama boyunca bir kez yapılır */
  let capsCache: Promise<Caps> | null = null;
</script>

<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { t as tr } from "./i18n.svelte";
  import LogModal from "./LogModal.svelte";
  import {
    LANGS, normLang, langName, codecName, chName, fmtSize, fmtDur, fmtBitrate, uid, api, baseName, charsetName,
    CHARSETS, SUB_EXT, QUALITY, VIDEO_CODECS, ENC_NAMES, HEIGHTS, AUDIO_EXT,
    type Item, type Rules, type Kind, type OutTrack, type Stream, type Convert, type AudioCodec, type Container,
    type Settings, type SubOpts, type VideoConv, type EncCaps, type SampleResult, type VideoCodec,
  } from "./types";
  import {
    derivedName, enforceContainer, normalize, signature, stereoConvert, outName, stem, NONE_ID, NO_SUB, subActive,
    attachExternal, detachExternal, defaultVideo,
  } from "./presets";

  let {
    item = $bindable(),
    presets,
    settings,
    onpreset,
    onnotice,
    actions,
  }: {
    item: Item;
    presets: Rules[];
    settings: Settings;
    onpreset: (id: string) => void;
    onnotice: (msg: string) => void;
    actions?: import("svelte").Snippet;
  } = $props();

  const bitrate = $derived(settings.bitrate);
  const nameTemplate = $derived(settings.nameTemplate);

  let tab = $state<"tracks" | "info">("tracks");

  const info = $derived(item.info!);
  const edit = $derived(item.edit!);
  const locked = $derived(item.status === "running" || item.status === "done");
  const modified = $derived(!!item.edit && signature(item.edit) !== item.base);
  const preset = $derived(presets.find((p) => p.id === item.presetId));

  const KINDS: { kind: Kind; title: string }[] = [
    { kind: "video", title: "Video" },
    { kind: "audio", title: "Ses" },
    { kind: "subtitle", title: "Altyazı" },
  ];

  const kindOf = (t: OutTrack) => info.streams[t.src].kind;
  const outOf = (k: Kind) => edit.tracks.filter((t) => kindOf(t) === k);
  /** Çıktıda olduğu gibi kopyası olmayan kaynak parçalar (çıkarılmış ya da yerine dönüştürülmüş) */
  const droppedOf = (k: Kind) =>
    info.streams.filter((s) => s.kind === k && !s.ext && !edit.tracks.some((t) => t.src === s.idx && !t.convert));
  /** Bulunmuş ama çıktıya eklenmemiş dış altyazılar */
  const extOff = $derived(info.streams.filter((s) => s.ext && !edit.tracks.some((t) => t.src === s.idx)));

  // ── Değişiklik yardımcıları
  function touch() {
    if (item.status !== "running") {
      item.status = "idle";
      item.error = "";
    }
  }

  function setKind(k: Kind, list: OutTrack[]) {
    const others = edit.tracks.filter((t) => kindOf(t) !== k);
    item.edit!.tracks = normalize(info, [...others, ...list]);
    touch();
  }

  function keepSource(s: Stream) {
    const list = outOf(s.kind);
    list.push({ uid: uid(), src: s.idx, convert: null, name: s.title, lang: normLang(s.lang), default: false, forced: s.forced });
    setKind(s.kind, list);
  }

  function removeTrack(t: OutTrack) {
    const k = kindOf(t);
    if (k === "audio" && outOf("audio").length === 1 && edit.tracks.length > 1) onnotice(tr("Dosyada hiç ses kalmayacak"));
    if (edit.tracks.length === 1) return onnotice(tr("En az bir parça kalmalı"));
    setKind(k, outOf(k).filter((x) => x.uid !== t.uid));
  }

  function derive(s: Stream, after?: OutTrack) {
    const c = stereoConvert(bitrate);
    const t: OutTrack = { uid: uid(), src: s.idx, convert: c, name: derivedName(s, c), lang: normLang(s.lang), default: false, forced: false };
    const list = outOf("audio");
    const at = after ? list.findIndex((x) => x.uid === after.uid) + 1 : list.length;
    list.splice(at, 0, t);
    setKind("audio", list);
  }

  function setMode(t: OutTrack, mode: "copy" | "convert") {
    const s = info.streams[t.src];
    if (mode === "convert") {
      t.convert = stereoConvert(bitrate);
      t.name = derivedName(s, t.convert);
    } else {
      if (edit.tracks.some((x) => x !== t && x.src === t.src && !x.convert)) return onnotice(tr("Bu parçanın orijinali zaten çıktıda"));
      t.convert = null;
      t.name = s.title;
    }
    touch();
  }

  function setConvert(t: OutTrack, patch: Partial<Convert>) {
    const s = info.streams[t.src];
    const auto = t.name === derivedName(s, t.convert!);
    t.convert = { ...t.convert!, ...patch };
    if (auto) t.name = derivedName(s, t.convert);
    touch();
  }

  function setDefault(t: OutTrack) {
    const k = kindOf(t);
    const on = !t.default;
    if (k !== "video" && on) for (const x of outOf(k)) x.default = false;
    t.default = on;
    touch();
  }

  function setContainer(c: Container) {
    item.edit!.container = c;
    const notes = enforceContainer(info, item.edit!);
    if (notes.length) onnotice(notes.join(" · "));
    touch();
  }

  // ── Sürükle-bırak ile sıralama (işaretçi olayları; Tauri'nin dosya bırakma özelliğiyle çakışmaz)
  let drag = $state<{ uid: string; kind: Kind } | null>(null);
  let listEls: Record<string, HTMLElement> = {};

  function gripDown(e: PointerEvent, t: OutTrack) {
    if (locked) return;
    drag = { uid: t.uid, kind: kindOf(t) };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function gripMove(e: PointerEvent) {
    if (!drag) return;
    const host = listEls[drag.kind];
    if (!host) return;
    const rows = [...host.querySelectorAll<HTMLElement>("[data-uid]")];
    const list = outOf(drag.kind);
    const from = list.findIndex((x) => x.uid === drag!.uid);
    let to = 0;
    for (const r of rows) {
      if (r.dataset.uid === drag.uid) continue;
      const b = r.getBoundingClientRect();
      if (e.clientY > b.top + b.height / 2) to++;
    }
    if (to !== from && from >= 0) {
      const [m] = list.splice(from, 1);
      list.splice(to, 0, m);
      setKind(drag.kind, list);
    }
  }
  function gripUp() {
    drag = null;
  }

  // ── Görünüm yardımcıları
  function srcLine(s: Stream) {
    if (s.kind === "video")
      return [s.width && s.height ? `${s.width}×${s.height}` : "", s.bitDepth ? `${s.bitDepth}-bit` : "", s.hdr, s.fps ? `${+s.fps.toFixed(3)} fps` : ""].filter(Boolean).join(" · ");
    if (s.kind === "audio") return [fmtBitrate(s.bitrate), s.sampleRate ? `${s.sampleRate / 1000} kHz` : ""].filter(Boolean).join(" · ");
    if (s.ext) return `${baseName(s.ext)}${s.textSub ? " · " + (s.charset ? tr(charsetName(s.charset)) : tr("UTF-8 değil")) : ""}`;
    return s.textSub ? tr("metin") : tr("görüntü");
  }
  function srcTitle(s: Stream) {
    return `${codecName(s.codec)}${s.kind === "audio" ? " " + chName(s.channels) : ""}`;
  }
  const langOptions = (cur: string) => (LANGS.some(([c]) => c === cur) ? LANGS : [[cur, cur.toUpperCase()] as [string, string], ...LANGS]);
  const dupOf = (s: Stream) =>
    info.streams.find(
      (b) => b.idx < s.idx && b.kind === "audio" && s.kind === "audio" && normLang(b.lang) === normLang(s.lang) && b.codec === s.codec && b.channels === s.channels && b.title.trim() === s.title.trim() && (!s.bitrate || !b.bitrate || s.bitrate === b.bitrate),
    );

  const CODECS: AudioCodec[] = ["aac", "ac3", "eac3", "opus", "flac", "mp3"];
  const CHANS = [
    [0, "Aynı"],
    [1, "Mono"],
    [2, "2.0"],
    [6, "5.1"],
    [8, "7.1"],
  ] as const;
  const RATES = [96, 128, 160, 192, 224, 256, 320, 384, 448, 640];

  const templateName = $derived(outName(item, nameTemplate));

  // ── Çıkarılan parçalar varsayılan olarak tek satıra katlanır
  let openDropped = $state<Record<string, boolean>>({});
  const KIND_WORD: Record<Kind, string> = { video: "video", audio: "ses", subtitle: "altyazı" };
  function droppedSummary(k: Kind, list: Stream[]) {
    const replaced = list.filter((s) => edit.tracks.some((t) => t.src === s.idx)).length;
    const removed = list.length - replaced;
    const parts = [];
    if (removed) parts.push(tr(`{n} ${KIND_WORD[k]} çıkarılacak`, { n: removed }));
    if (replaced) parts.push(tr("{n} tanesinin yerine dönüştürülmüş hali kullanılacak", { n: replaced }));
    return parts.join(" · ");
  }
  // ── Altyazı işlemleri
  let openSub = $state<Record<string, boolean>>({});
  let openExt = $state(true);
  let textView = $state<{ title: string; load: () => Promise<string> } | null>(null);
  const snap = () => $state.snapshot(settings) as Settings;
  const optsOf = (t?: OutTrack): SubOpts => ({ ...NO_SUB, ...t?.sub });

  function setSub(t: OutTrack, patch: Partial<SubOpts>) {
    const o = { ...optsOf(t), ...patch };
    if (subActive(o)) t.sub = o;
    else delete t.sub;
    touch();
  }

  function subSummary(t: OutTrack, s: Stream): string {
    const o = optsOf(t);
    const parts: string[] = [];
    if (o.charset) parts.push(o.charset.toUpperCase());
    if (o.fixTr) parts.push(tr("TR düzelt"));
    if (o.shift) parts.push(`${o.shift > 0 ? "+" : ""}${+(o.shift / 1000).toFixed(3)} ${tr("sn")}`);
    if (o.toSrt || (s.codec === "webvtt" && edit.container === "mkv")) parts.push("→ SRT");
    if (!parts.length && s.ext && s.textSub && !s.charset) parts.push("→ UTF-8");
    if (!parts.length) return edit.container === "mp4" && s.textSub ? tr("→ MP4 metin") : tr("Olduğu gibi");
    return parts.join(" · ");
  }

  const dirOf = (p: string) => p.slice(0, Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);

  async function addSubFiles() {
    const r = await open({ multiple: true, defaultPath: dirOf(item.path) || undefined, filters: [{ name: tr("Altyazı"), extensions: SUB_EXT }] });
    if (!r) return;
    try {
      const list = await api.probeSubs(snap(), item.path, Array.isArray(r) ? r : [r]);
      const n = attachExternal(item, list, true);
      onnotice(n ? tr("{n} altyazı eklendi", { n }) : tr("Bu altyazı zaten listede"));
      touch();
    } catch (e) {
      onnotice(String(e));
    }
  }

  function removeExt(path: string) {
    detachExternal(item, path);
    touch();
  }

  function preview(s: Stream, t?: OutTrack) {
    const opts = optsOf(t);
    textView = {
      title: `${tr("Önizleme")} · #${s.idx + 1} ${codecName(s.codec)}${s.ext ? " · " + baseName(s.ext) : ""}`,
      load: () => api.previewSub(snap(), item.path, $state.snapshot(s) as Stream, opts),
    };
  }

  async function exportSub(s: Stream, t?: OutTrack) {
    const o = optsOf(t);
    const ext = !s.textSub
      ? s.codec === "dvd_subtitle" ? "idx" : "sup"
      : (s.codec === "ass" || s.codec === "ssa") && !o.toSrt ? "ass"
      : s.codec === "webvtt" && !o.toSrt ? "vtt"
      : "srt";
    const out = await save({
      defaultPath: `${dirOf(item.path)}${stem(item.name)}.${normLang(t?.lang ?? s.lang)}.${ext}`,
      filters: [{ name: tr("Altyazı"), extensions: [ext] }],
    });
    if (!out) return;
    onnotice(tr("Dışa aktarılıyor…"));
    try {
      const done = await api.exportSub(snap(), item.path, $state.snapshot(s) as Stream, o, out);
      onnotice(tr("Kaydedildi: {ad}", { ad: baseName(done) }));
    } catch (e) {
      onnotice(String(e));
    }
  }

  // ── Video dönüştürme
  let caps = $state<EncCaps | null>(null);
  let samples = $state<Record<string, { busy: boolean; res?: SampleResult; err?: string }>>({});

  function loadCaps() {
    capsCache ??= api.detectEncoders(snap());
    capsCache.then((c) => (caps = c)).catch((e) => {
      capsCache = null;
      onnotice(String(e));
    });
  }
  if (item.edit?.tracks.some((x) => x.video)) loadCaps();

  function setVideoMode(t: OutTrack, mode: "copy" | "convert") {
    const s = info.streams[t.src];
    if (mode === "convert") {
      t.video = defaultVideo(s, s.codec === "h264" ? "hevc" : "h264");
      openSub[t.uid] = true;
      loadCaps();
    } else {
      delete t.video;
    }
    touch();
  }

  function setVideo(t: OutTrack, patch: Partial<VideoConv>) {
    const s = info.streams[t.src];
    const v = { ...t.video!, ...patch };
    if (patch.codec) {
      v.quality = QUALITY[patch.codec][1][0];
      v.to8bit = patch.codec === "h264" || s.bitDepth <= 8;
      v.tonemap = !!s.hdr && patch.codec === "h264";
    }
    t.video = v;
    delete samples[t.uid];
    touch();
  }

  const famOk = (codec: VideoCodec, fam: string) => !caps || fam === "auto" || caps[codec].includes(fam);
  const autoFam = (codec: VideoCodec) => caps?.[codec][0];

  function videoSummary(v: VideoConv): string {
    const parts = [codecName(v.codec)];
    const fam = v.encoder === "auto" ? autoFam(v.codec) : v.encoder;
    if (fam) parts.push(fam === "cpu" ? tr("İşlemci") : fam.toUpperCase());
    if (v.maxHeight) parts.push(`${v.maxHeight}p`);
    if (v.tonemap) parts.push("SDR");
    if (v.burn != null) parts.push(tr("altyazı yakılı"));
    return parts.join(" · ");
  }

  async function runSample(t: OutTrack) {
    samples[t.uid] = { busy: true };
    try {
      const res = await api.sampleEncode(snap(), item.path, t.src, $state.snapshot(t.video!) as VideoConv);
      samples[t.uid] = { busy: false, res };
    } catch (e) {
      samples[t.uid] = { busy: false, err: String(e) };
    }
  }

  // ── Ses ekstraları
  const audioActive = (t: OutTrack) => !!t.delay || !!t.convert?.normalize || !!t.convert?.dialog;

  function setDelay(t: OutTrack, ms: number) {
    if (ms) t.delay = ms;
    else delete t.delay;
    touch();
  }

  async function exportAudio(s: Stream) {
    const ext = AUDIO_EXT[s.codec] ?? "mka";
    const out = await save({
      defaultPath: `${dirOf(item.path)}${stem(item.name)}.${normLang(s.lang)}.${ext}`,
      filters: [{ name: tr("Ses"), extensions: [ext] }],
    });
    if (!out) return;
    onnotice(tr("Dışa aktarılıyor…"));
    try {
      const done = await api.exportAudio(snap(), item.path, s.ffIndex, out);
      onnotice(tr("Kaydedildi: {ad}", { ad: baseName(done) }));
    } catch (e) {
      onnotice(String(e));
    }
  }

  function restoreAll(k: Kind, list: Stream[]) {
    const out = outOf(k);
    for (const s of list) {
      if (edit.container === "mp4" && k === "subtitle" && !s.textSub) continue;
      out.push({ uid: uid(), src: s.idx, convert: null, name: s.title, lang: normLang(s.lang), default: false, forced: s.forced });
    }
    setKind(k, out);
  }
</script>

<svelte:window onpointerup={gripUp} />

<section class="editor" class:locked>
  <header class="fh">
    <div class="tline">
      <div class="title" title={item.path}>{item.name}</div>
      {#if actions}<div class="acts">{@render actions()}</div>{/if}
    </div>
    <div class="meta">
      <span>{info.format}</span>
      <span>{fmtSize(info.size || item.size)}</span>
      {#if info.duration}<span>{fmtDur(info.duration)}</span>{/if}
      {#if info.bitrate}<span>{tr("toplam {x}", { x: fmtBitrate(info.bitrate) })}</span>{/if}
      <span>{tr("{b} bölüm işareti · {e} ek", { b: info.chapters, e: info.attachments })}</span>
    </div>
    {#if item.status === "error" && item.error}
      <div class="alert err">{item.error}</div>
    {:else if item.status === "done"}
      <div class="alert ok">{tr("İşlendi →")} <span class="mono">{item.output}</span></div>
    {/if}
    <div class="bar">
      <span class="muted">{tr("Hazır ayar")}</span>
      <select value={item.presetId} onchange={(e) => onpreset((e.currentTarget as HTMLSelectElement).value)} disabled={locked}>
        {#each presets as p (p.id)}
          <option value={p.id}>{tr(p.name)}</option>
        {/each}
      </select>
      {#if modified}
        <span class="chip warn">{tr("elle değiştirildi")}</span>
        <button class="sm ghost" onclick={() => onpreset(item.presetId)} disabled={locked}>{tr("Sıfırla")}</button>
      {/if}
      <span class="sp"></span>
      <div class="seg">
        <button class:on={tab === "tracks"} onclick={() => (tab = "tracks")}>{tr("Parçalar")}</button>
        <button class:on={tab === "info"} onclick={() => (tab = "info")}>{tr("Dosya bilgisi")}</button>
      </div>
    </div>
    {#if preset && preset.id !== NONE_ID && !modified}
      <div class="hint pdesc">{tr(preset.desc)}</div>
    {/if}
  </header>

  {#if tab === "tracks"}
    <div class="tracks">
      <div class="row hd">
        <span></span><span title={tr("Tut")}>✓</span><span>{tr("Parça")}</span><span>{tr("Ad")}</span><span>{tr("Dil")}</span><span>{tr("İşlem")}</span><span>{tr("Vars.")}</span><span>{tr("Zor.")}</span><span></span>
      </div>

      {#each KINDS as { kind, title } (kind)}
        {@const outs = outOf(kind)}
        {@const dropped = droppedOf(kind)}
        {#if outs.length || dropped.length || kind === "subtitle"}
          <div class="sec">
            {tr(title)}
            {#if kind === "subtitle" && !locked}
              <button class="sm ghost addsub" onclick={addSubFiles} title={tr("Ya da .srt dosyasını pencereye sürükle")}>{tr("+ Altyazı dosyası ekle")}</button>
            {/if}
          </div>
          <div class="list" bind:this={listEls[kind]}>
            {#each outs as t (t.uid)}
              {@const s = info.streams[t.src]}
              <div class="row" class:conv={!!t.convert || !!t.video} class:dragging={drag?.uid === t.uid} data-uid={t.uid}>
                <span class="grip" title={tr("Sürükleyerek sırala")} onpointerdown={(e) => gripDown(e, t)} onpointermove={gripMove} role="button" tabindex="-1">⋮⋮</span>
                <input type="checkbox" checked onchange={() => removeTrack(t)} disabled={locked} title={tr("Çıktıdan çıkar")} />
                <span class="c">
                  {#if t.convert}
                    <b>{codecName(t.convert.codec)} {t.convert.channels ? chName(t.convert.channels) : chName(s.channels)}</b><span class="chip accent">{tr("yeni")}</span>
                    <span class="d">{tr("#{n} {src} parçasından", { n: s.idx + 1, src: srcTitle(s) })}</span>
                  {:else if t.video}
                    <b>{codecName(t.video.codec)}{t.video.maxHeight ? ` ${t.video.maxHeight}p` : ""}</b><span class="chip accent">{tr("yeni")}</span>
                    <span class="d">{tr("#{n} {src} parçasından", { n: s.idx + 1, src: srcTitle(s) })} · {srcLine(s)}</span>
                  {:else}
                    <b>#{s.idx + 1} {srcTitle(s)}</b>{#if s.ext}<span class="chip">{tr("dış dosya")}</span>{/if}
                    <span class="d" title={s.ext || undefined}>{srcLine(s)}</span>
                  {/if}
                </span>
                <input bind:value={t.name} oninput={touch} disabled={locked} placeholder={tr("(adsız)")} />
                <select bind:value={t.lang} onchange={touch} disabled={locked}>
                  {#each langOptions(t.lang) as [c, n] (c)}
                    <option value={c}>{c} — {tr(n)}</option>
                  {/each}
                </select>
                <div class="act">
                  {#if kind === "audio" && t.convert}
                    <select value={t.convert.codec} onchange={(e) => setConvert(t, { codec: (e.currentTarget as HTMLSelectElement).value as AudioCodec })} disabled={locked} title={tr("Codec")}>
                      {#each CODECS as c}<option value={c}>{codecName(c)}</option>{/each}
                    </select>
                    <select value={t.convert.channels} onchange={(e) => setConvert(t, { channels: +(e.currentTarget as HTMLSelectElement).value })} disabled={locked} title={tr("Kanal")}>
                      {#each CHANS as [v, n]}<option value={v}>{tr(n)}</option>{/each}
                    </select>
                    {#if t.convert.codec !== "flac"}
                      <select value={t.convert.bitrate} onchange={(e) => setConvert(t, { bitrate: +(e.currentTarget as HTMLSelectElement).value })} disabled={locked} title={tr("Bit hızı (kbps)")}>
                        {#each RATES as r}<option value={r}>{r}k</option>{/each}
                      </select>
                    {/if}
                  {:else if kind === "audio"}
                    <select value="copy" onchange={(e) => setMode(t, (e.currentTarget as HTMLSelectElement).value as "copy" | "convert")} disabled={locked}>
                      <option value="copy">{tr("Olduğu gibi")}</option>
                      <option value="convert">{tr("Dönüştür…")}</option>
                    </select>
                  {:else if kind === "video" && t.video}
                    <button class="sm ghost subbtn on" onclick={() => (openSub[t.uid] = !openSub[t.uid])} aria-expanded={!!openSub[t.uid]}>
                      <span class="sum">{videoSummary(t.video)}</span><span class="caret" class:open={openSub[t.uid]}>▸</span>
                    </button>
                  {:else if kind === "video" && !["mjpeg", "png"].includes(s.codec)}
                    <select value="copy" onchange={(e) => setVideoMode(t, (e.currentTarget as HTMLSelectElement).value as "copy" | "convert")} disabled={locked}>
                      <option value="copy">{tr("Olduğu gibi")}</option>
                      <option value="convert">{tr("Dönüştür…")}</option>
                    </select>
                  {:else if kind === "subtitle"}
                    <button class="sm ghost subbtn" class:on={subActive(t.sub)} onclick={() => (openSub[t.uid] = !openSub[t.uid])} aria-expanded={!!openSub[t.uid]} title={tr("Kodlama, Türkçe karakter, zaman kaydırma, dışa aktarma")}>
                      <span class="sum">{subSummary(t, s)}</span><span class="caret" class:open={openSub[t.uid]}>▸</span>
                    </button>
                  {:else}
                    <span class="muted">{tr("Olduğu gibi")}</span>
                  {/if}
                </div>
                <button class="toggle" class:on={t.default} onclick={() => setDefault(t)} disabled={locked} aria-label={tr("Varsayılan")}></button>
                <button class="toggle" class:on={t.forced} onclick={() => { t.forced = !t.forced; touch(); }} disabled={locked || kind === "video"} aria-label={tr("Zorunlu")}></button>
                <span class="end">
                  {#if kind === "audio"}
                    <button class="sm ghost more" class:on={audioActive(t)} onclick={() => (openSub[t.uid] = !openSub[t.uid])} title={tr("Gecikme, ses eşitleme, diyalog, dışa aktarma")} aria-expanded={!!openSub[t.uid]}>⋯</button>
                  {/if}
                  {#if kind === "audio" && t.convert}
                    <button class="sm ghost" onclick={() => removeTrack(t)} disabled={locked}>{tr("Kaldır")}</button>
                  {:else if kind === "audio"}
                    <button class="sm ghost" onclick={() => derive(s, t)} disabled={locked} title={tr("Bu parçadan yeni bir ses parçası türet (ör. stereo AAC)")}>{tr("+ Türet")}</button>
                  {/if}
                </span>
              </div>
              {#if kind === "video" && t.video && openSub[t.uid]}
                {@const v = t.video}
                {@const sm = samples[t.uid]}
                <div class="subopt vopt">
                  <label>
                    <span class="muted">{tr("Codec")}</span>
                    <select value={v.codec} onchange={(e) => setVideo(t, { codec: (e.currentTarget as HTMLSelectElement).value as VideoCodec })} disabled={locked}>
                      {#each VIDEO_CODECS as [c, n] (c)}<option value={c} disabled={!!caps && !caps[c].length}>{n}</option>{/each}
                    </select>
                  </label>
                  <label>
                    <span class="muted">{tr("Kodlayıcı")}</span>
                    <select value={v.encoder} onchange={(e) => setVideo(t, { encoder: (e.currentTarget as HTMLSelectElement).value as VideoConv["encoder"] })} disabled={locked}>
                      {#each Object.entries(ENC_NAMES) as [f, n] (f)}
                        <option value={f} disabled={!famOk(v.codec, f)}>
                          {tr(n)}{f === "auto" && autoFam(v.codec) ? ` (${autoFam(v.codec) === "cpu" ? tr("İşlemci") : autoFam(v.codec)!.toUpperCase()})` : ""}{!famOk(v.codec, f) ? ` — ${tr("yok")}` : ""}
                        </option>
                      {/each}
                    </select>
                  </label>
                  <label>
                    <span class="muted">{tr("Kalite")}</span>
                    <select value={v.quality} onchange={(e) => setVideo(t, { quality: +(e.currentTarget as HTMLSelectElement).value })} disabled={locked}>
                      {#each QUALITY[v.codec] as [q, n] (q)}<option value={q}>{tr(n)}</option>{/each}
                      {#if !QUALITY[v.codec].some(([q]) => q === v.quality)}<option value={v.quality}>{v.quality}</option>{/if}
                    </select>
                  </label>
                  <label>
                    <span class="muted">{tr("Hız")}</span>
                    <select value={v.speed} onchange={(e) => setVideo(t, { speed: (e.currentTarget as HTMLSelectElement).value as VideoConv["speed"] })} disabled={locked}>
                      <option value="fast">{tr("Hızlı")}</option>
                      <option value="medium">{tr("Orta")}</option>
                      <option value="slow">{tr("Yavaş (daha küçük)")}</option>
                    </select>
                  </label>
                  <label>
                    <span class="muted">{tr("Çözünürlük")}</span>
                    <select value={v.maxHeight} onchange={(e) => setVideo(t, { maxHeight: +(e.currentTarget as HTMLSelectElement).value })} disabled={locked}>
                      <option value={0}>{tr("Aynı")} ({s.height}p)</option>
                      {#each HEIGHTS.filter((h) => h < s.height) as h (h)}<option value={h}>{h}p</option>{/each}
                    </select>
                  </label>
                  {#if s.hdr}
                    <label class="chk" title={s.hdr === "Dolby Vision" ? tr("Dolby Vision profil 5 dosyalarda renkler doğru çıkmayabilir") : ""}>
                      <input type="checkbox" checked={v.tonemap} onchange={(e) => setVideo(t, { tonemap: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                      {tr("HDR → SDR")} <span class="hint">({s.hdr})</span>
                    </label>
                  {/if}
                  {#if s.bitDepth > 8 && v.codec !== "h264" && !v.tonemap}
                    <label class="chk">
                      <input type="checkbox" checked={v.to8bit} onchange={(e) => setVideo(t, { to8bit: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                      {tr("8-bit'e indir")}
                    </label>
                  {/if}
                  <label>
                    <span class="muted">{tr("Altyazı yak")}</span>
                    <select value={v.burn ?? ""} onchange={(e) => { const x = (e.currentTarget as HTMLSelectElement).value; setVideo(t, { burn: x === "" ? null : +x }); }} disabled={locked}>
                      <option value="">{tr("Yok")}</option>
                      {#each info.streams.filter((x) => x.kind === "subtitle") as x (x.idx)}
                        <option value={x.idx}>#{x.idx + 1} {codecName(x.codec)} · {normLang(x.lang)}{x.title ? ` · ${x.title}` : ""}{x.ext ? ` · ${tr("dış")}` : ""}</option>
                      {/each}
                    </select>
                  </label>
                  <span class="sp"></span>
                  <button class="sm ghost" onclick={() => runSample(t)} disabled={sm?.busy} title={tr("Filmin ortasından 30 saniye kodlar; hız ve tahmini boyutu gösterir")}>
                    {sm?.busy ? tr("Deneniyor…") : tr("30 sn dene")}
                  </button>
                  <button class="sm ghost" onclick={() => setVideoMode(t, "copy")} disabled={locked}>{tr("Olduğu gibi bırak")}</button>
                  {#if sm?.res}
                    <div class="sample">
                      {tr("Deneme: {sn} sn video {boyut} · hız {hiz}× · tahmini video boyutu {tahmin} (kaynak dosya {kaynak})", {
                        sn: 30, boyut: fmtSize(sm.res.bytes), hiz: sm.res.speed.toFixed(1), tahmin: fmtSize(sm.res.estBytes), kaynak: fmtSize(info.size || item.size),
                      })}
                      · {sm.res.encoder === "cpu" ? tr("İşlemci") : sm.res.encoder.toUpperCase()}
                      <button class="sm ghost" onclick={() => api.openPath(sm.res!.path)}>{tr("İzle")}</button>
                    </div>
                  {:else if sm?.err}
                    <div class="sample err">{sm.err}</div>
                  {/if}
                </div>
              {/if}
              {#if kind === "audio" && openSub[t.uid]}
                <div class="subopt">
                  <label>
                    <span class="muted">{tr("Gecikme")}</span>
                    <input class="shift" type="number" step="0.05" value={(t.delay ?? 0) / 1000}
                      onchange={(e) => setDelay(t, Math.round(+(e.currentTarget as HTMLInputElement).value * 1000) || 0)} disabled={locked} />
                    <span class="muted">{tr("sn")}</span>
                    <span class="hint">{tr("(artı sesi geciktirir)")}</span>
                  </label>
                  {#if t.convert}
                    <label class="chk">
                      <input type="checkbox" checked={!!t.convert.normalize} onchange={(e) => setConvert(t, { normalize: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                      {tr("Ses seviyesini eşitle")} <span class="hint">{tr("(EBU R128)")}</span>
                    </label>
                    {#if s.channels > 2 && t.convert.channels === 2}
                      <label class="chk">
                        <input type="checkbox" checked={!!t.convert.dialog} onchange={(e) => setConvert(t, { dialog: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                        {tr("Diyaloğu öne çıkar")} <span class="hint">{tr("(5.1'de kısık konuşma sorunu)")}</span>
                      </label>
                    {/if}
                  {:else if edit.container === "mp4" && t.delay}
                    <span class="hint warn">{tr("MP4'te kopyalanan seste gecikme yapılamaz; sesi dönüştür ya da MKV seç.")}</span>
                  {:else}
                    <span class="hint">{tr("Eşitleme ve diyalog için sesi dönüştür.")}</span>
                  {/if}
                  <span class="sp"></span>
                  {#if !t.convert}<button class="sm ghost" onclick={() => exportAudio(s)}>{tr("Dışa aktar…")}</button>{/if}
                </div>
              {/if}
              {#if kind === "subtitle" && openSub[t.uid]}
                {@const o = optsOf(t)}
                <div class="subopt">
                  {#if s.textSub && s.ext}
                    <label>
                      <span class="muted">{tr("Kodlama")}</span>
                      <select value={o.charset} onchange={(e) => setSub(t, { charset: (e.currentTarget as HTMLSelectElement).value })} disabled={locked}>
                        <option value="">{tr("Otomatik")} — {s.charset ? tr(charsetName(s.charset)) : tr("{x} sayılır", { x: tr(charsetName(settings.subCharset)) })}</option>
                        {#each CHARSETS as [c, n] (c)}<option value={c}>{tr(n)}</option>{/each}
                      </select>
                    </label>
                  {/if}
                  {#if s.textSub}
                    <label class="chk">
                      <input type="checkbox" checked={o.fixTr} onchange={(e) => setSub(t, { fixTr: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                      {tr("Bozuk Türkçe karakterleri düzelt")} <span class="hint">{tr("(ý þ ð → ı ş ğ)")}</span>
                    </label>
                    {#if (s.codec === "ass" || s.codec === "ssa") && edit.container === "mkv"}
                      <label class="chk">
                        <input type="checkbox" checked={o.toSrt} onchange={(e) => setSub(t, { toSrt: (e.currentTarget as HTMLInputElement).checked })} disabled={locked} />
                        {tr("SRT'ye çevir")} <span class="hint">{tr("(stil ve konumlar kaybolur)")}</span>
                      </label>
                    {/if}
                  {/if}
                  {#if s.textSub || edit.container === "mkv"}
                    <label>
                      <span class="muted">{tr("Kaydır")}</span>
                      <input
                        class="shift"
                        type="number"
                        step="0.1"
                        value={o.shift / 1000}
                        onchange={(e) => setSub(t, { shift: Math.round(+(e.currentTarget as HTMLInputElement).value * 1000) || 0 })}
                        disabled={locked}
                      />
                      <span class="muted">{tr("sn")}</span>
                      <span class="hint">{tr("(artı geciktirir, eksi öne alır)")}</span>
                    </label>
                  {/if}
                  <span class="sp"></span>
                  {#if s.textSub}<button class="sm ghost" onclick={() => preview(s, t)}>{tr("Önizle")}</button>{/if}
                  {#if !(s.ext && !s.textSub)}<button class="sm ghost" onclick={() => exportSub(s, t)}>{tr("Dışa aktar…")}</button>{/if}
                </div>
              {/if}
            {/each}
          </div>
          {#if dropped.length}
            <div class="fold">
              <button class="fold-btn" onclick={() => (openDropped[kind] = !openDropped[kind])} aria-expanded={!!openDropped[kind]}>
                <span class="caret" class:open={openDropped[kind]}>▸</span>
                {droppedSummary(kind, dropped)}
              </button>
              {#if openDropped[kind] && !locked}
                <button class="sm ghost" onclick={() => restoreAll(kind, dropped)}>{tr("Hepsini geri ekle")}</button>
              {/if}
            </div>
          {/if}
          {#each openDropped[kind] ? dropped : [] as s (s.idx)}
            {@const dup = dupOf(s)}
            <div class="row off">
              <span></span>
              <input type="checkbox" onchange={() => keepSource(s)} disabled={locked || (edit.container === "mp4" && kind === "subtitle" && !s.textSub)} title={tr("Çıktıya ekle")} />
              <span class="c">
                <b>#{s.idx + 1} {srcTitle(s)}</b>
                {#if dup}<span class="chip err">{tr("#{n} ile aynı", { n: dup.idx + 1 })}</span>{:else if edit.container === "mp4" && kind === "subtitle" && !s.textSub}<span class="chip">{tr("MP4'e konamaz")}</span>{/if}
                <span class="d">{srcLine(s)}</span>
              </span>
              <input value={s.title} disabled placeholder={tr("(adsız)")} />
              <select disabled><option>{normLang(s.lang)} — {tr(langName(s.lang))}</option></select>
              <span class="muted act">{edit.tracks.some((t) => t.src === s.idx) ? tr("Dönüştürülmüş hali var") : tr("Çıkarılacak")}</span>
              <span></span><span></span>
              <span class="end">
                {#if kind === "audio"}
                  <button class="sm ghost" onclick={() => derive(s)} disabled={locked}>{tr("+ Türet")}</button>
                {:else if kind === "subtitle"}
                  <button class="sm ghost" onclick={() => exportSub(s)} title={tr("Bu altyazıyı ayrı dosya olarak kaydet")}>{tr("Dışa aktar")}</button>
                {/if}
              </span>
            </div>
          {/each}
          {#if kind === "subtitle" && extOff.length}
            <div class="fold">
              <button class="fold-btn" onclick={() => (openExt = !openExt)} aria-expanded={openExt}>
                <span class="caret" class:open={openExt}>▸</span>
                {tr("{n} dış altyazı bulundu, çıktıya eklenmedi", { n: extOff.length })}
              </button>
              {#if openExt && !locked}
                <button class="sm ghost" onclick={() => restoreAll("subtitle", extOff)}>{tr("Hepsini ekle")}</button>
              {/if}
            </div>
            {#each openExt ? extOff : [] as s (s.idx)}
              <div class="row off ext">
                <span></span>
                <input type="checkbox" onchange={() => keepSource(s)} disabled={locked || (edit.container === "mp4" && !s.textSub)} title={tr("Çıktıya ekle")} />
                <span class="c">
                  <b>#{s.idx + 1} {srcTitle(s)}</b><span class="chip">{tr("dış dosya")}</span>
                  {#if edit.container === "mp4" && !s.textSub}<span class="chip">{tr("MP4'e konamaz")}</span>{/if}
                  <span class="d" title={s.ext}>{srcLine(s)}</span>
                </span>
                <input value={s.title} disabled placeholder={tr("(adsız)")} />
                <select disabled><option>{normLang(s.lang)} — {tr(langName(s.lang))}</option></select>
                <span class="act">
                  {#if s.textSub}<button class="sm ghost" onclick={() => preview(s)}>{tr("Önizle")}</button>{/if}
                </span>
                <span></span><span></span>
                <span class="end">
                  <button class="sm ghost" onclick={() => removeExt(s.ext!)} disabled={locked} title={tr("Listeden kaldır (dosya silinmez)")}>{tr("Kaldır")}</button>
                </span>
              </div>
            {/each}
          {/if}
        {/if}
      {/each}

      <div class="sec">{tr("Çıktı")}</div>
      <div class="out">
        <label>
          <span class="muted">{tr("Kapsayıcı")}</span>
          <select value={edit.container} onchange={(e) => setContainer((e.currentTarget as HTMLSelectElement).value as Container)} disabled={locked}>
            <option value="mkv">{tr("MKV")}</option>
            <option value="mp4">{tr("MP4")}</option>
          </select>
        </label>
        <label class="chk"><input type="checkbox" bind:checked={item.edit!.keepChapters} onchange={touch} disabled={locked || !info.chapters} /> {tr("Bölüm işaretleri")} ({info.chapters})</label>
        <label class="chk"><input type="checkbox" bind:checked={item.edit!.keepAttachments} onchange={touch} disabled={locked || !info.attachments || edit.container === "mp4"} /> {tr("Ekler")} ({info.attachments})</label>
        <label class="chk"><input type="checkbox" bind:checked={item.edit!.clearTitle} onchange={touch} disabled={locked} /> {tr("Dosya başlığını temizle")}{info.title ? ` (“${info.title}”)` : ""}</label>
        <label class="name">
          <span class="muted">{tr("Çıktı adı")}</span>
          <input
            class="mono"
            value={item.nameOverride ?? ""}
            oninput={(e) => { item.nameOverride = (e.currentTarget as HTMLInputElement).value; touch(); }}
            placeholder={templateName}
            disabled={locked}
            title={tr("Boş bırakırsan alttaki ad şablonu kullanılır")}
          />
          <span class="mono ext">.{edit.container}</span>
          {#if item.nameOverride?.trim()}
            <button class="sm ghost" onclick={() => { item.nameOverride = ""; touch(); }} disabled={locked}>{tr("Şablona dön")}</button>
          {/if}
        </label>
      </div>
    </div>
  {:else}
    <div class="tracks">
      <dl class="info">
        <dt>{tr("Konum")}</dt><dd class="mono">{item.path}</dd>
        <dt>{tr("Kapsayıcı")}</dt><dd>{info.format}{info.title ? ` · ${tr("başlık")} “${info.title}”` : ""}</dd>
        <dt>{tr("Boyut / Süre")}</dt><dd>{fmtSize(info.size || item.size)} · {fmtDur(info.duration) || "—"}{info.bitrate ? ` · ${tr("toplam {x}", { x: fmtBitrate(info.bitrate) })}` : ""}</dd>
        {#each KINDS as { kind, title } (kind)}
          {@const list = info.streams.filter((s) => s.kind === kind)}
          <dt>{tr(title)} ({list.length})</dt>
          <dd>
            {#each list as s (s.idx)}
              <div>
                #{s.idx + 1}
                {codecName(s.codec)}{s.profile ? ` (${s.profile})` : ""}
                {#if kind === "video"}· {srcLine(s)}{s.bitrate ? ` · ${fmtBitrate(s.bitrate)}` : ""}{/if}
                {#if kind === "audio"}· {chName(s.channels)}{s.layout ? ` (${s.layout})` : ""} · {srcLine(s)}{/if}
                {#if kind !== "video"}· {tr(langName(s.lang))}{/if}
                {#if s.ext}· {tr("dış dosya")}: {baseName(s.ext)}{/if}
                {s.title ? ` · “${s.title}”` : ""}{s.isDefault ? ` · ${tr("varsayılan")}` : ""}{s.forced ? ` · ${tr("zorunlu")}` : ""}
              </div>
            {:else}
              <div class="muted">{tr("Yok")}</div>
            {/each}
          </dd>
        {/each}
        <dt>{tr("Ekler / Bölümler")}</dt><dd>{tr("{e} ek (font, kapak…) · {b} bölüm işareti", { e: info.attachments, b: info.chapters })}</dd>
      </dl>
    </div>
  {/if}
</section>

{#if textView}
  <LogModal title={textView.title} load={textView.load} hint={tr("Karakterler bozuksa kodlamayı değiştirip tekrar önizle.")} onclose={() => (textView = null)} />
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .fh {
    padding: 14px 18px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  .tline {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .acts {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: text;
  }
  .meta {
    color: var(--muted);
    font-size: 12px;
    margin-top: 3px;
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .alert {
    margin-top: 8px;
    padding: 7px 10px;
    border-radius: 8px;
    font-size: 12.5px;
    user-select: text;
    word-break: break-all;
  }
  .alert.err {
    background: var(--err-bg);
    color: var(--err);
  }
  .alert.ok {
    background: var(--ok-bg);
    color: var(--ok);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    flex-wrap: wrap;
  }
  .bar .sp {
    flex: 1;
  }
  .pdesc {
    margin-top: 7px;
  }
  .tracks {
    flex: 1;
    overflow: auto;
    padding: 6px 18px 18px;
  }
  .locked .tracks {
    opacity: 0.75;
  }
  .sec {
    font-size: 11px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.07em;
    margin: 14px 0 6px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sec::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--line-soft);
  }
  .row {
    display: grid;
    grid-template-columns: 14px 18px minmax(130px, 1.1fr) minmax(110px, 1fr) 150px 228px 32px 32px 104px;
    gap: 8px;
    align-items: center;
    padding: 6px 8px;
    border-radius: 9px;
    background: var(--panel);
    border: 1px solid var(--line-soft);
    margin-bottom: 4px;
  }
  .row.hd {
    background: none;
    border: 0;
    color: var(--muted);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 6px 8px 0;
    margin: 0;
    position: sticky;
    top: -6px;
    z-index: 2;
    background: var(--bg);
  }
  .row.conv {
    border-color: rgb(139 124 246 / 0.4);
    background: linear-gradient(90deg, rgb(139 124 246 / 0.09), var(--panel) 45%);
  }
  .row.off {
    opacity: 0.5;
    border-style: dashed;
  }
  .row.off b {
    text-decoration: line-through;
  }
  .row.off.ext {
    opacity: 0.75;
  }
  .row.off.ext b {
    text-decoration: none;
  }
  .sec .addsub {
    order: 2;
    text-transform: none;
    letter-spacing: 0;
    padding: 2px 8px;
    font-size: 11.5px;
  }
  .subbtn {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12px;
    color: var(--muted);
  }
  .subbtn.on {
    color: var(--accent);
    border-color: rgb(139 124 246 / 0.4);
  }
  .subbtn .sum {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .subopt {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
    margin: -2px 0 6px 40px;
    padding: 9px 12px;
    border: 1px solid var(--line-soft);
    border-top: 0;
    border-radius: 0 0 9px 9px;
    background: var(--panel-2);
    font-size: 12.5px;
  }
  .subopt label {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .subopt select {
    max-width: 260px;
    padding: 4px 6px;
  }
  .subopt .shift {
    width: 76px;
    padding: 4px 6px;
  }
  .subopt .sp {
    flex: 1;
  }
  .end {
    display: flex;
    justify-content: flex-end;
    gap: 4px;
  }
  .end .more {
    padding: 3px 6px;
    letter-spacing: 1px;
  }
  .end .more.on {
    color: var(--accent);
    border-color: rgb(139 124 246 / 0.4);
  }
  .vopt select {
    max-width: 190px;
  }
  .sample {
    flex-basis: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ok);
    user-select: text;
  }
  .sample.err {
    color: var(--err);
  }
  .hint.warn {
    color: var(--warn);
  }
  .row.dragging {
    border-color: var(--accent);
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.35);
    position: relative;
    z-index: 3;
  }
  .grip {
    color: var(--muted);
    cursor: grab;
    letter-spacing: -2px;
    touch-action: none;
  }
  .row.dragging .grip {
    cursor: grabbing;
  }
  .c {
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 6px;
  }
  .c b {
    font-weight: 600;
    white-space: nowrap;
  }
  .c .d {
    color: var(--muted);
    font-size: 11.5px;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row input:not([type="checkbox"]),
  .row select {
    width: 100%;
    padding: 4px 7px;
  }
  .act {
    display: flex;
    gap: 4px;
    min-width: 0;
    font-size: 12.5px;
  }
  .act select {
    padding: 4px 4px;
  }
  .toggle {
    margin: auto;
  }
  .end {
    text-align: right;
  }
  .end button {
    padding: 3px 7px;
    font-size: 12px;
    white-space: nowrap;
  }
  .out {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 18px;
    padding: 4px 8px;
    font-size: 12.5px;
  }
  .out label {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .out .name {
    flex-basis: 100%;
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .out .name input {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
  }
  .out .name .ext {
    color: var(--muted);
  }
  .fold {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 2px 0 4px;
  }
  .fold-btn {
    flex: 1;
    text-align: left;
    background: transparent;
    border: 1px dashed var(--line);
    color: var(--muted);
    padding: 6px 10px;
    font-size: 12.5px;
  }
  .fold-btn:hover:not(:disabled) {
    color: var(--text);
  }
  .caret {
    display: inline-block;
    width: 12px;
    transition: transform 0.15s;
  }
  .caret.open {
    transform: rotate(90deg);
  }
  .info {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 8px 14px;
    margin: 12px 0 0;
    font-size: 13px;
  }
  .info dt {
    color: var(--muted);
  }
  .info dd {
    margin: 0;
    user-select: text;
    line-height: 1.6;
  }
</style>
