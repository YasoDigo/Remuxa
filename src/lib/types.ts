import { invoke } from "@tauri-apps/api/core";
import { t, i18n } from "./i18n.svelte";

// ───────────────────────── Ayarlar ─────────────────────────

export type OutMode = "local" | "target" | "inplace";
export type Target = { name: string; path: string };

export type Settings = {
  ffmpegPath: string;
  ffprobePath: string;
  mkvmergePath: string;
  outMode: OutMode;
  outTarget: string;
  localDir: string;
  targets: Target[];
  workDir: string;
  bitrate: number;
  prefLangs: string;
  overwrite: boolean;
  keepStructure: boolean;
  nameTemplate: string;
  notify: boolean;
  /** UTF-8 olmayan altyazılar için varsayılan kodlama */
  subCharset: string;
  /** "" = sistem diline göre, "tr" | "en" */
  lang: string;
  /** Açılışta yeni sürüm denetle */
  checkUpdates: boolean;
  presets: Rules[];
};

export const DEFAULT_SETTINGS: Settings = {
  ffmpegPath: "", ffprobePath: "", mkvmergePath: "", outMode: "local", outTarget: "", localDir: "",
  targets: [], workDir: "", bitrate: 192, prefLangs: "tur,jpn,eng", overwrite: false, keepStructure: true,
  nameTemplate: "{ad}", notify: true, subCharset: "windows-1254", lang: "", checkUpdates: true, presets: [],
};

export type ToolStatus = {
  ffmpeg: string | null;
  ffprobe: string | null;
  mkvmerge: string | null;
};

// ───────────────────────── İnceleme ─────────────────────────

export type Kind = "video" | "audio" | "subtitle";

export type Stream = {
  idx: number;
  ffIndex: number;
  tid: number;
  kind: Kind;
  codec: string;
  profile: string;
  lang: string;
  title: string;
  isDefault: boolean;
  forced: boolean;
  bitrate: number;
  channels: number;
  layout: string;
  sampleRate: number;
  width: number;
  height: number;
  fps: number;
  bitDepth: number;
  hdr: string;
  colorPrimaries?: string;
  colorTransfer?: string;
  colorSpace?: string;
  textSub: boolean;
  /** Dış dosyadan geliyorsa yolu */
  ext?: string;
  /** Dış metin altyazının kodlaması: "utf-8" | "utf-16le" | "utf-16be" | "" (eski kodlama) */
  charset?: string;
};

export type ProbeInfo = {
  duration: number;
  size: number;
  format: string;
  container: string;
  bitrate: number;
  title: string;
  streams: Stream[];
  attachments: number;
  chapters: number;
};

export type InputFile = { path: string; rel: string; size: number };

// ───────────────────────── Çıktı planı ─────────────────────────

export type AudioCodec = "aac" | "ac3" | "eac3" | "opus" | "flac" | "mp3";
export type Convert = {
  codec: AudioCodec;
  channels: number;
  bitrate: number;
  /** Ses seviyesini eşitle (EBU R128) */
  normalize?: boolean;
  /** 5.1 → stereo inerken diyaloğu öne çıkar */
  dialog?: boolean;
};

export type VideoCodec = "h264" | "hevc" | "av1";
export type EncFamily = "auto" | "cpu" | "nvenc" | "qsv" | "amf";
export type VideoConv = {
  codec: VideoCodec;
  encoder: EncFamily;
  /** CRF benzeri: düşük = daha iyi */
  quality: number;
  speed: "fast" | "medium" | "slow";
  /** 0 = kaynakla aynı */
  maxHeight: number;
  tonemap: boolean;
  to8bit: boolean;
  /** Görüntüye yakılacak altyazının Stream.idx değeri */
  burn: number | null;
};
export type EncCaps = { h264: string[]; hevc: string[]; av1: string[]; zscale: boolean; libass: boolean };
export type SampleResult = { path: string; seconds: number; bytes: number; estBytes: number; speed: number; encoder: string };

/** Altyazı işlemleri */
export type SubOpts = {
  /** Boş = otomatik */
  charset: string;
  fixTr: boolean;
  /** ms */
  shift: number;
  toSrt: boolean;
};

export type OutTrack = {
  uid: string;
  src: number;
  convert: Convert | null;
  name: string;
  lang: string;
  default: boolean;
  forced: boolean;
  /** Yalnızca altyazılarda; varsayılan değerlerdeyken hiç yazılmaz */
  sub?: SubOpts;
  /** Yalnızca videoda: yeniden kodla */
  video?: VideoConv;
  /** Yalnızca seste: gecikme (ms) */
  delay?: number;
};

export type Container = "mkv" | "mp4";

export type Edit = {
  tracks: OutTrack[];
  container: Container;
  keepChapters: boolean;
  keepAttachments: boolean;
  clearTitle: boolean;
};

// ───────────────────────── Hazır ayarlar ─────────────────────────

export type Rules = {
  id: string;
  name: string;
  desc: string;
  builtin?: boolean;
  /** Birebir aynı ses parçalarının fazlasını çıkar */
  dropDuplicates: boolean;
  /** Başlığında "commentary/yorum" geçen ses ve altyazıları çıkar */
  dropCommentary: boolean;
  /** Ses dilleri: hepsi / yalnızca tercih edilenler */
  audioLangs: "all" | "pref";
  /** Altyazılar: hepsi / tercih edilen diller / hiçbiri */
  subLangs: "all" | "pref" | "none";
  /** Uyumsuz ses (AC3, DTS, 5.1…): dokunma / yanına stereo AAC ekle / stereo AAC ile değiştir */
  stereo: "none" | "add" | "replace";
  stereoBitrate: number;
  /** Varsayılan parçalar: olduğu gibi / dil önceliğine göre */
  defaults: "keep" | "pref";
  container: "keep" | Container;
  keepChapters: boolean;
  keepAttachments: boolean;
  clearTitle: boolean;
  /** Videonun yanındaki altyazı dosyalarını ekle */
  sidecars?: boolean;
  /** Metin altyazılarda bozuk Türkçe karakterleri düzelt */
  fixTr?: boolean;
  /** Video: olduğu gibi ya da bu codec'e çevir */
  videoCodec?: "keep" | VideoCodec;
  /** Video en fazla bu yükseklikte (0 = aynı) */
  videoMaxHeight?: number;
  /** Dönüştürülen seslerde ses seviyesini eşitle */
  normalize?: boolean;
  /** Stereo dönüşümlerde diyaloğu öne çıkar */
  dialog?: boolean;
};

// ───────────────────────── Kuyruk ─────────────────────────

export type Status = "probing" | "idle" | "running" | "done" | "error" | "cancelled";

export type Item = {
  id: string;
  path: string;
  rel: string;
  name: string;
  size: number;
  info: ProbeInfo | null;
  edit: Edit | null;
  /** Son uygulanan hazır ayarın ürettiği plan (elle değişiklik tespiti için) */
  base: string;
  presetId: string;
  status: Status;
  stage: string;
  percent: number;
  error: string;
  output: string;
  checked: boolean;
  /** Bu dosyaya özel çıktı adı (uzantısız). Boşsa alttaki ad şablonu kullanılır. */
  nameOverride?: string;
  /** Son çalıştırmanın log kimliği */
  logId?: string;
};

export type HistoryEntry = {
  id: string;
  time: number;
  source: string;
  output: string;
  mode: OutMode;
  status: "ok" | "error" | "cancelled";
  seconds: number;
  error: string;
  encoded: number;
};

export type Job = {
  id: string;
  path: string;
  rel: string;
  mode: OutMode;
  targetDir: string;
  container: Container;
  outName: string;
  externals: string[];
  tracks: Omit<OutTrack, "uid">[];
  keepChapters: boolean;
  keepAttachments: boolean;
  clearTitle: boolean;
};

/** Rust tarafı hata mesajlarını etkin dilde versin */
const S = (s: Settings): Settings => ({ ...s, lang: i18n.lang });

export const api = {
  loadSettings: () => invoke<Settings>("load_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  checkTools: (settings: Settings) => invoke<ToolStatus>("check_tools", { settings: S(settings) }),
  expandPaths: (paths: string[]) => invoke<InputFile[]>("expand_paths", { paths }),
  probe: (settings: Settings, path: string) => invoke<ProbeInfo>("probe", { settings: S(settings), path }),
  convert: (settings: Settings, job: Job) => invoke<{ output: string; encoded: number; seconds: number }>("convert", { settings: S(settings), job }),
  cancel: () => invoke<void>("cancel"),
  readText: (path: string) => invoke<string>("read_text", { path }),
  writeText: (path: string, text: string) => invoke<void>("write_text", { path, text }),
  saveQueue: (text: string) => invoke<void>("save_queue", { text }),
  loadQueue: () => invoke<string>("load_queue"),
  loadHistory: () => invoke<HistoryEntry[]>("load_history"),
  clearHistory: () => invoke<void>("clear_history"),
  readJobLog: (id: string) => invoke<string>("read_job_log", { id }),
  probeSubs: (settings: Settings, video: string, paths: string[]) => invoke<Stream[]>("probe_subs", { settings: S(settings), video, paths }),
  previewSub: (settings: Settings, video: string, stream: Stream, opts: SubOpts) =>
    invoke<string>("preview_sub", { settings: S(settings), video, stream, opts }),
  exportSub: (settings: Settings, video: string, stream: Stream, opts: SubOpts, out: string) =>
    invoke<string>("export_sub", { settings: S(settings), video, stream, opts, out }),
  detectEncoders: (settings: Settings) => invoke<EncCaps>("detect_encoders", { settings: S(settings) }),
  sampleEncode: (settings: Settings, path: string, stream: number, video: VideoConv) =>
    invoke<SampleResult>("sample_encode", { settings: S(settings), path, stream, video }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  installTools: (settings: Settings, which: string[]) => invoke<string>("install_tools", { settings: S(settings), which }),
  exportAudio: (settings: Settings, video: string, ffIndex: number, out: string) =>
    invoke<string>("export_audio", { settings: S(settings), video, ffIndex, out }),
};

/** Kalite seçenekleri (codec'e göre CRF benzeri değerler) */
export const QUALITY: Record<VideoCodec, [number, string][]> = {
  h264: [[18, "Çok yüksek"], [20, "Yüksek"], [23, "Dengeli"], [26, "Küçük dosya"]],
  hevc: [[20, "Çok yüksek"], [22, "Yüksek"], [25, "Dengeli"], [28, "Küçük dosya"]],
  av1: [[24, "Çok yüksek"], [28, "Yüksek"], [32, "Dengeli"], [38, "Küçük dosya"]],
};
export const VIDEO_CODECS: [VideoCodec, string][] = [["h264", "H.264"], ["hevc", "HEVC (H.265)"], ["av1", "AV1"]];
export const ENC_NAMES: Record<string, string> = {
  auto: "Otomatik", cpu: "İşlemci", nvenc: "NVIDIA (NVENC)", qsv: "Intel (QSV)", amf: "AMD (AMF)",
};
export const HEIGHTS = [2160, 1440, 1080, 720, 480];
/** Sesin kayıpsız dışa aktarımında dosya uzantısı */
export const AUDIO_EXT: Record<string, string> = {
  aac: "m4a", ac3: "ac3", eac3: "eac3", dts: "dts", truehd: "thd", flac: "flac", opus: "opus", mp3: "mp3",
};

export const SUB_EXT = ["srt", "ass", "ssa", "vtt", "sup", "idx"];
export const isSubPath = (p: string) => SUB_EXT.includes(p.split(".").pop()?.toLowerCase() ?? "");

export const CHARSETS: [string, string][] = [
  ["utf-8", "UTF-8"],
  ["windows-1254", "Türkçe (Windows-1254)"],
  ["iso-8859-9", "Türkçe (ISO-8859-9)"],
  ["windows-1252", "Batı Avrupa (Windows-1252)"],
  ["windows-1250", "Orta Avrupa (Windows-1250)"],
  ["windows-1251", "Kiril (Windows-1251)"],
  ["windows-1253", "Yunanca (Windows-1253)"],
  ["windows-1255", "İbranice (Windows-1255)"],
  ["windows-1256", "Arapça (Windows-1256)"],
  ["gbk", "Çince basit (GBK)"],
  ["big5", "Çince geleneksel (Big5)"],
  ["shift_jis", "Japonca (Shift_JIS)"],
  ["euc-kr", "Korece (EUC-KR)"],
];
export const charsetName = (c: string) => CHARSETS.find(([k]) => k === c)?.[1] ?? c;

export const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

export const STAGES: Record<string, string> = {
  probe: "İnceleniyor",
  audio: "Ses kodlanıyor",
  mux: "Birleştiriliyor",
  subs: "Altyazılar hazırlanıyor",
  video: "Video kodlanıyor",
  verify: "Doğrulanıyor",
  copy: "Hedefe kopyalanıyor",
};

// ───────────────────────── Biçimlendirme ─────────────────────────

export const LANGS: [string, string][] = [
  ["und", "Belirsiz"], ["tur", "Türkçe"], ["eng", "İngilizce"], ["jpn", "Japonca"], ["ger", "Almanca"],
  ["fre", "Fransızca"], ["spa", "İspanyolca"], ["ita", "İtalyanca"], ["rus", "Rusça"], ["por", "Portekizce"],
  ["kor", "Korece"], ["chi", "Çince"], ["ara", "Arapça"], ["hin", "Hintçe"], ["dut", "Felemenkçe"],
  ["pol", "Lehçe"], ["swe", "İsveççe"], ["nor", "Norveççe"], ["dan", "Danca"], ["fin", "Fince"],
  ["gre", "Yunanca"], ["heb", "İbranice"], ["per", "Farsça"], ["aze", "Azerbaycanca"], ["ukr", "Ukraynaca"],
];
const LANG_ALIAS: Record<string, string> = { deu: "ger", fra: "fre", zho: "chi", nld: "dut", ell: "gre", fas: "per", tr: "tur", en: "eng", ja: "jpn" };
export const normLang = (l: string) => {
  const x = (l || "und").toLowerCase();
  return LANG_ALIAS[x] ?? x;
};
export const langName = (l: string) => LANGS.find(([c]) => c === normLang(l))?.[1] ?? l.toUpperCase();

export function chName(ch: number): string {
  if (ch === 1) return "Mono";
  if (ch === 2) return "2.0";
  if (ch === 6) return "5.1";
  if (ch === 8) return "7.1";
  return ch ? `${ch}ch` : "";
}

export const CODEC_NAMES: Record<string, string> = {
  aac: "AAC", ac3: "AC3", eac3: "E-AC3", dts: "DTS", truehd: "TrueHD", opus: "Opus", flac: "FLAC", mp3: "MP3",
  vorbis: "Vorbis", pcm_s16le: "PCM", pcm_s24le: "PCM", h264: "H.264", hevc: "HEVC", av1: "AV1", vp9: "VP9",
  mpeg4: "MPEG-4", mpeg2video: "MPEG-2", subrip: "SRT", ass: "ASS", ssa: "SSA", webvtt: "WebVTT",
  mov_text: "MP4 Text", hdmv_pgs_subtitle: "PGS", dvd_subtitle: "VobSub", dvb_subtitle: "DVB",
};
export const codecName = (c: string) => CODEC_NAMES[c] ?? c.toUpperCase();

export function fmtSize(b: number): string {
  if (b >= 1e9) return (b / 1e9).toFixed(2) + " GB";
  if (b >= 1e6) return (b / 1e6).toFixed(0) + " MB";
  return (b / 1e3).toFixed(0) + " KB";
}

export function fmtDur(s: number): string {
  if (!s) return "";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? t("{h}sa {m}dk", { h, m }) : t("{m}dk", { m });
}

export function fmtBitrate(b: number): string {
  if (!b) return "";
  if (b >= 1e6) return (b / 1e6).toFixed(1) + " Mbps";
  return Math.round(b / 1e3) + " kbps";
}

export const uid = () => Math.random().toString(36).slice(2, 10);
