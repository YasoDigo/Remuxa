import {
  normLang, uid, chName, codecName,
  type Edit, type OutTrack, type ProbeInfo, type Rules, type Stream, type Container, type Convert, type Item,
  type SubOpts, type Job, type VideoConv, type VideoCodec, QUALITY,
} from "./types";
import { t as tl } from "./i18n.svelte";

// ───────────────────────── Yerleşik hazır ayarlar ─────────────────────────

const base: Omit<Rules, "id" | "name" | "desc"> = {
  dropDuplicates: false, dropCommentary: false, audioLangs: "all", subLangs: "all", stereo: "none",
  stereoBitrate: 192, defaults: "keep", container: "keep", keepChapters: true, keepAttachments: true, clearTitle: false,
  sidecars: false, fixTr: false, videoCodec: "keep", videoMaxHeight: 0, normalize: false, dialog: false,
};

export const NONE_ID = "none";

export const BUILTIN: Rules[] = [
  { ...base, id: NONE_ID, builtin: true, name: "Olduğu gibi", desc: "Hiçbir şey değiştirme; düzenlemeleri elle yap." },
  {
    ...base, id: "media-server", builtin: true, name: "Medya sunucusu uyumlu ses",
    desc: "Jellyfin / Plex / Emby için: uyumsuz sese (AC3, DTS, 5.1) stereo AAC kopya ekler, varsayılanı dil önceliğine göre seçer, birebir kopyaları atar. Tarayıcı ve telefonlarda transcode biter.",
    dropDuplicates: true, stereo: "add", defaults: "pref",
  },
  {
    ...base, id: "clean", builtin: true, name: "Gereksiz parçaları temizle",
    desc: "Yorum (commentary) ses/altyazılarını ve birebir kopya ses parçalarını çıkarır.",
    dropDuplicates: true, dropCommentary: true,
  },
  {
    ...base, id: "pref-langs", builtin: true, name: "Sadece tercih ettiğim diller",
    desc: "Dil önceliğindeki diller dışındaki ses ve altyazıları çıkarır, varsayılanları ayarlar. Eşleşen ses yoksa sesler korunur.",
    dropDuplicates: true, dropCommentary: true, audioLangs: "pref", subLangs: "pref", defaults: "pref",
  },
  {
    ...base, id: "stereo-replace", builtin: true, name: "Tüm sesi stereo AAC yap",
    desc: "Uyumsuz ses parçalarını stereo AAC ile değiştirir. Dosya küçülür, her cihazda çalar; 5.1 kaybolur.",
    dropDuplicates: true, stereo: "replace", defaults: "pref",
  },
  {
    ...base, id: "fix-defaults", builtin: true, name: "Varsayılan dili düzelt",
    desc: "Sadece varsayılan ses/altyazı bayraklarını dil önceliğine göre ayarlar. Video yanlış dilde açılıyorsa.",
    defaults: "pref",
  },
  {
    ...base, id: "embed-subs", builtin: true, name: "Yanındaki altyazıları içine göm",
    desc: "Videonun yanındaki aynı adlı .srt / .ass / .vtt / .sup dosyalarını dosyanın içine ekler; UTF-8 olmayanları çevirir, bozuk Türkçe karakterleri düzeltir. Diğer parçalar olduğu gibi kalır.",
    sidecars: true, fixTr: true,
  },
  {
    ...base, id: "h264-compat", builtin: true, name: "Eski cihazlar için H.264",
    desc: "HEVC / AV1 videoyu H.264 1080p SDR'ye çevirir, uyumsuz sese stereo AAC ekler. Eski TV, tablet ve tarayıcılarda transcode olmadan oynar. Video kodlandığı için uzun sürer; ekran kartı varsa onu kullanır.",
    dropDuplicates: true, stereo: "add", defaults: "pref", videoCodec: "h264", videoMaxHeight: 1080,
  },
  {
    ...base, id: "shrink-hevc", builtin: true, name: "Arşiv için küçült (HEVC)",
    desc: "H.264 videoyu HEVC'ye çevirerek dosyayı yaklaşık yarıya indirir. HDR ve 10-bit korunur, ses ve altyazılar olduğu gibi kalır.",
    videoCodec: "hevc",
  },
  {
    ...base, id: "night", builtin: true, name: "Gece izleme (diyalog + eşit ses)",
    desc: "5.1 sesi diyaloğu öne çıkaran stereo AAC'ye çevirir ve ses seviyesini eşitler. Patlamalar kısılır, konuşmalar duyulur.",
    dropDuplicates: true, stereo: "add", stereoBitrate: 192, defaults: "pref", normalize: true, dialog: true,
  },
  {
    ...base, id: "mp4-mobile", builtin: true, name: "Telefon / tarayıcı için MP4",
    desc: "MP4'e çevirir: sesi stereo AAC yapar, görüntü altyazılarını ve ekleri çıkarır, metin altyazıları MP4 formatına çevirir.",
    dropDuplicates: true, dropCommentary: true, stereo: "replace", defaults: "pref", container: "mp4", keepAttachments: false,
  },
];

export const blankRules = (): Rules => ({ ...base, id: "u-" + uid(), name: "Yeni hazır ayar", desc: "" });

// ───────────────────────── Yardımcılar ─────────────────────────

export const parsePrefs = (s: string) => s.split(/[\s,;]+/).map((x) => normLang(x.trim())).filter(Boolean);

const isCommentary = (s: Stream) => /comment|yorum|director/i.test(s.title);
const isCompatAudio = (s: Stream) => s.codec === "aac" && s.channels > 0 && s.channels <= 2;
/** MP4'e olduğu gibi konabilen ses codec'leri */
const MP4_AUDIO = new Set(["aac", "ac3", "eac3", "mp3", "opus", "flac", "alac"]);

/** Önceki bir ses parçasının birebir kopyası mı? (eski scriptin eklediği gibi) */
export function isDuplicate(streams: Stream[], s: Stream): boolean {
  if (s.kind !== "audio") return false;
  return streams.some(
    (b) =>
      b.idx < s.idx && b.kind === "audio" &&
      normLang(b.lang) === normLang(s.lang) && b.codec === s.codec && b.channels === s.channels &&
      b.title.trim().toLowerCase() === s.title.trim().toLowerCase() &&
      (!s.bitrate || !b.bitrate || s.bitrate === b.bitrate),
  );
}

/** "Japanese 5.1" -> "Japanese Stereo", boş başlık -> "JPN Stereo" */
export function derivedName(s: Stream, c: Convert): string {
  const words = s.title
    .split(/\s+/)
    .filter((w) => !/^[([]?(5\.1|7\.1|6\.1|2\.0|6ch|8ch|surround|stereo|dts|ac3|e-?ac3|truehd|atmos|aac|dd\+?|ddp)[)\]]?$/i.test(w));
  const ch = c.channels === 2 ? "Stereo" : c.channels ? chName(c.channels) : chName(s.channels);
  const label = `${ch} ${codecName(c.codec)}`.trim();
  const head = words.join(" ").trim() || (normLang(s.lang) === "und" ? "" : normLang(s.lang).toUpperCase());
  return head ? `${head} ${label}` : label;
}

export const stereoConvert = (bitrate: number): Convert => ({ codec: "aac", channels: 2, bitrate });

/** Video dönüştürme için başlangıç ayarı */
export function defaultVideo(s: Stream, codec: VideoCodec = "h264", maxHeight = 0): VideoConv {
  return {
    codec,
    encoder: "auto",
    quality: QUALITY[codec][1][0],
    speed: "medium",
    maxHeight: maxHeight && s.height > maxHeight ? maxHeight : 0,
    tonemap: !!s.hdr && codec === "h264",
    to8bit: codec === "h264" || s.bitDepth <= 8,
    burn: null,
  };
}

function copyOf(s: Stream): OutTrack {
  return { uid: uid(), src: s.idx, convert: null, name: s.title, lang: normLang(s.lang), default: s.isDefault, forced: s.forced };
}

function convertedOf(s: Stream, c: Convert): OutTrack {
  return { uid: uid(), src: s.idx, convert: { ...c }, name: derivedName(s, c), lang: normLang(s.lang), default: false, forced: false };
}

const KIND_ORDER = { video: 0, audio: 1, subtitle: 2 } as const;
export function kindOf(info: ProbeInfo, t: OutTrack) {
  return info.streams[t.src].kind;
}
/** Parçaları tür gruplarına göre (video, ses, altyazı) sırala; grup içi sıra korunur */
export function normalize(info: ProbeInfo, tracks: OutTrack[]): OutTrack[] {
  return tracks
    .map((t, i) => ({ t, i }))
    .sort((a, b) => KIND_ORDER[kindOf(info, a.t)] - KIND_ORDER[kindOf(info, b.t)] || a.i - b.i)
    .map((x) => x.t);
}

// ───────────────────────── Plan üretimi ─────────────────────────

export function identity(info: ProbeInfo): Edit {
  return {
    tracks: info.streams.filter((s) => !s.ext).map(copyOf),
    container: info.container === "mp4" ? "mp4" : "mkv",
    keepChapters: true,
    keepAttachments: true,
    clearTitle: false,
  };
}

export function applyRules(info: ProbeInfo, r: Rules, prefLangs: string): Edit {
  const prefs = parsePrefs(prefLangs);
  const S = info.streams;
  const container: Container = r.container === "keep" ? (info.container === "mp4" ? "mp4" : "mkv") : r.container;
  const out: OutTrack[] = [];

  // Video: olduğu gibi ya da istenen codec'e (zaten o codec'teyse ve küçültme gerekmiyorsa dokunulmaz)
  const vc = r.videoCodec && r.videoCodec !== "keep" ? r.videoCodec : null;
  for (const s of S.filter((s) => s.kind === "video")) {
    const t = copyOf(s);
    const tooBig = !!r.videoMaxHeight && s.height > r.videoMaxHeight;
    const isCover = s.codec === "mjpeg" || s.codec === "png";
    if (vc && !isCover && (s.codec !== vc || tooBig || (vc === "h264" && !!s.hdr))) t.video = defaultVideo(s, vc, r.videoMaxHeight ?? 0);
    out.push(t);
  }

  // Ses
  let audio = S.filter((s) => s.kind === "audio");
  audio = audio.filter((s) => !(r.dropDuplicates && isDuplicate(S, s)) && !(r.dropCommentary && isCommentary(s)));
  if (r.audioLangs === "pref") {
    const keep = audio.filter((s) => prefs.includes(normLang(s.lang)));
    if (keep.length) audio = keep; // hiç eşleşme yoksa sessiz dosya üretme
  }
  const compatLangs = new Set(audio.filter(isCompatAudio).map((s) => normLang(s.lang)));
  const added = new Set<string>();
  const conv: Convert = { ...stereoConvert(r.stereoBitrate || 192), ...(r.normalize ? { normalize: true } : {}), ...(r.dialog ? { dialog: true } : {}) };
  for (const s of audio) {
    const lang = normLang(s.lang);
    if (r.stereo === "replace" && !isCompatAudio(s)) {
      out.push(convertedOf(s, conv));
      continue;
    }
    out.push(copyOf(s));
    if (r.stereo === "add" && !isCompatAudio(s) && !compatLangs.has(lang) && !added.has(lang)) {
      out.push(convertedOf(s, conv));
      added.add(lang);
    }
  }

  // Altyazı (dış dosyalar yalnızca istenirse)
  if (r.subLangs !== "none") {
    let subs = S.filter((s) => s.kind === "subtitle" && (!s.ext || r.sidecars) && !(r.dropCommentary && isCommentary(s)));
    if (r.subLangs === "pref") subs = subs.filter((s) => prefs.includes(normLang(s.lang)));
    for (const s of subs) {
      const t = copyOf(s);
      if (s.ext) t.default = false; // gömülü varsayılanla çakışmasın
      if (r.fixTr && s.textSub) t.sub = { ...NO_SUB, fixTr: true };
      out.push(t);
    }
  }

  const edit: Edit = {
    tracks: out,
    container,
    keepChapters: r.keepChapters,
    keepAttachments: r.keepAttachments,
    clearTitle: r.clearTitle,
  };
  if (r.defaults === "pref") applyPrefDefaults(info, edit, prefs);
  enforceContainer(info, edit);
  return edit;
}

/** Varsayılan sesi dil önceliğine göre seç (aynı dilde uyumlu stereo varsa onu),
 *  ses tercih edilen ilk dil değilse o dildeki altyazıyı varsayılan yap. */
export function applyPrefDefaults(info: ProbeInfo, edit: Edit, prefs: string[]) {
  const audio = edit.tracks.filter((t) => kindOf(info, t) === "audio");
  const subs = edit.tracks.filter((t) => kindOf(info, t) === "subtitle");
  const compat = (t: OutTrack) => (t.convert ? t.convert.codec === "aac" && t.convert.channels === 2 : isCompatAudio(info.streams[t.src]));
  let pick: OutTrack | undefined;
  for (const p of prefs) {
    const same = audio.filter((t) => t.lang === p);
    pick = same.find(compat) ?? same[0];
    if (pick) break;
  }
  pick ??= audio.find((t) => t.default) ?? audio.find(compat) ?? audio[0];
  for (const t of audio) t.default = t === pick;

  for (const t of subs) t.default = false;
  const first = prefs[0];
  if (pick && first && pick.lang !== first) {
    const s = subs.find((t) => t.lang === first && !t.forced) ?? subs.find((t) => t.lang === first);
    if (s) s.default = true;
  }
}

/** Kapsayıcının kaldıramadığı şeyleri düzelt; ne yapıldığını açıklayan uyarılar döner */
export function enforceContainer(info: ProbeInfo, edit: Edit): string[] {
  const notes: string[] = [];
  if (edit.container !== "mp4") return notes;
  const before = edit.tracks.length;
  edit.tracks = edit.tracks.filter((t) => {
    const s = info.streams[t.src];
    return !(s.kind === "subtitle" && !s.textSub);
  });
  if (edit.tracks.length < before) notes.push(tl("{n} görüntü altyazısı MP4'e konamadığı için çıkarıldı", { n: before - edit.tracks.length }));
  for (const t of edit.tracks) {
    const s = info.streams[t.src];
    if (s.kind === "audio" && !t.convert && !MP4_AUDIO.has(s.codec)) {
      t.convert = { codec: "aac", channels: 0, bitrate: s.channels > 2 ? 384 : 192 };
      t.name = t.name || derivedName(s, t.convert);
      notes.push(tl("{c} ses MP4 için AAC'ye çevrilecek", { c: codecName(s.codec) }));
    }
  }
  if (edit.keepAttachments && info.attachments) {
    edit.keepAttachments = false;
    notes.push(tl("Ekler (font, kapak) MP4'e konamaz"));
  }
  return notes;
}

/** MP4'e geçerken yapılacak değişiklikleri önceden göster */
export function mp4Problems(info: ProbeInfo, edit: Edit): string[] {
  const copy: Edit = JSON.parse(JSON.stringify(edit));
  copy.container = "mp4";
  return enforceContainer(info, copy);
}

// ───────────────────────── Altyazı ─────────────────────────

export const NO_SUB: SubOpts = { charset: "", fixTr: false, shift: 0, toSrt: false };
export const subActive = (o?: SubOpts) => !!o && (!!o.charset || o.fixTr || !!o.shift || o.toSrt);

/** Dış altyazı parçalarını dosyaya ekle; `keep` ise çıktıya da konur */
export function attachExternal(it: Item, list: Stream[], keep: boolean): number {
  if (!it.info || !it.edit) return 0;
  const have = new Set(it.info.streams.filter((s) => s.ext).map((s) => s.ext));
  let n = 0;
  for (const s of list) {
    if (have.has(s.ext)) continue;
    const st = { ...s, idx: it.info.streams.length };
    it.info.streams.push(st);
    if (keep && !(it.edit.container === "mp4" && !st.textSub)) {
      it.edit.tracks.push({ ...copyOf(st), default: false });
    }
    n++;
  }
  it.edit.tracks = normalize(it.info, it.edit.tracks);
  return n;
}

/** Dış dosyayı listeden tamamen kaldır; sonraki parçaların numaraları kayar */
export function detachExternal(it: Item, path: string) {
  if (!it.info || !it.edit) return;
  const keep = it.info.streams.filter((s) => s.ext !== path);
  const map = new Map<number, number>();
  keep.forEach((s, i) => map.set(s.idx, i));
  it.edit.tracks = it.edit.tracks.filter((t) => map.has(t.src)).map((t) => ({ ...t, src: map.get(t.src)! }));
  it.info.streams = keep.map((s, i) => ({ ...s, idx: i }));
}

/** İş için parça listesi: yalnızca kullanılan dış dosyalar gönderilir, numaralar buna göre kaydırılır */
export function jobTracks(info: ProbeInfo, edit: Edit): Pick<Job, "externals" | "tracks"> {
  const used = new Set(edit.tracks.map((t) => info.streams[t.src].ext).filter(Boolean) as string[]);
  const externals: string[] = [];
  for (const s of info.streams) if (s.ext && used.has(s.ext) && !externals.includes(s.ext)) externals.push(s.ext);
  const map = new Map<number, number>();
  let n = 0;
  for (const s of info.streams) if (!s.ext) map.set(s.idx, n++);
  for (const p of externals) for (const s of info.streams) if (s.ext === p) map.set(s.idx, n++);
  return {
    externals,
    tracks: edit.tracks.map(({ uid: _u, ...t }) => ({ ...t, src: map.get(t.src)! })),
  };
}

// ───────────────────────── Karşılaştırma ─────────────────────────

/** Planın karşılaştırılabilir hali (uid'ler olmadan) */
export function signature(e: Edit): string {
  return JSON.stringify({ ...e, tracks: e.tracks.map(({ uid: _u, ...t }) => t) });
}

export function isIdentity(info: ProbeInfo, e: Edit): boolean {
  return signature(e) === signature(identity(info));
}

/** Dosyada yapılacak gerçek bir iş var mı? */
export function hasWork(it: Item, nameTemplate: string): boolean {
  if (!it.info || !it.edit) return false;
  return (
    !isIdentity(it.info, it.edit) ||
    it.info.container !== it.edit.container ||
    finalName(it, nameTemplate) !== stem(it.name)
  );
}

/** Aynı yapıdaki dosyalar: tür, dil, codec ve kanal sırası aynı */
export function layoutKey(info: ProbeInfo): string {
  return info.streams.map((s) => `${s.ext ? "dış:" : ""}${s.kind}:${normLang(s.lang)}:${s.codec}:${s.channels}`).join("|");
}

/** Bir dosyanın planını aynı yapıdaki başka dosyaya kopyala */
export function cloneEdit(e: Edit): Edit {
  const c: Edit = JSON.parse(JSON.stringify(e));
  for (const t of c.tracks) t.uid = uid();
  return c;
}

// ───────────────────────── Dosya adı şablonu ─────────────────────────

export const stem = (name: string) => name.replace(/\.[^.]+$/, "");

/** Kullanılacak çıktı adı: dosyaya özel ad yazıldıysa o, yoksa şablon */
export function finalName(it: Pick<Item, "name" | "rel" | "nameOverride">, template: string): string {
  return it.nameOverride?.trim() || outName(it, template);
}

/** {ad} {dizi} {sezon} {bolum} {klasor} */
export function outName(it: Pick<Item, "name" | "rel">, template: string): string {
  const ad = stem(it.name);
  const t = (template || "{ad}").trim() || "{ad}";
  const m = ad.match(/^(.*?)[\s._-]*S(\d{1,2})[\s._-]*E(\d{1,3})/i);
  const parts = it.rel.split("/");
  const klasor = parts.length > 1 ? parts[parts.length - 2] : "";
  const dizi = m ? m[1].replace(/[._]+/g, " ").trim() : (parts.length > 2 ? parts[parts.length - 3] : klasor) || ad;
  const sezon = m ? m[2].padStart(2, "0") : "";
  const bolum = m ? m[3].padStart(2, "0") : "";
  // Türkçe ve İngilizce etiketler
  const tags: [string[], string][] = [
    [["{ad}", "{name}"], ad],
    [["{dizi}", "{series}"], dizi],
    [["{sezon}", "{season}"], sezon],
    [["{bolum}", "{episode}"], bolum],
    [["{klasor}", "{folder}"], klasor],
  ];
  let out = t;
  for (const [keys, val] of tags) for (const k of keys) out = out.replaceAll(k, val);
  return out.trim() || ad;
}
