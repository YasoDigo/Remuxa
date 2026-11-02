use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};

/// Arayüz dili: hata mesajları da bu dilde döner
static LANG_EN: AtomicBool = AtomicBool::new(false);

fn set_lang(s: &Settings) {
    LANG_EN.store(s.lang == "en", Ordering::Relaxed);
}

fn en() -> bool {
    LANG_EN.load(Ordering::Relaxed)
}

/// Türkçe / İngilizce mesaj: m!("Türkçe {x}", "English {x}")
macro_rules! m {
    ($tr:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if crate::en() { format!($en $(, $a)*) } else { format!($tr $(, $a)*) }
    };
}

mod subs;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "ts", "m2ts", "mov", "webm"];
const TMP_EXT: &str = "remuxa-tmp";
const CANCELLED: &str = "İptal edildi";
const TEXT_SUBS: &[&str] = &["subrip", "ass", "ssa", "webvtt", "mov_text", "text", "srt"];

// ───────────────────────── Ayarlar ─────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Target {
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub mkvmerge_path: String,
    /// "local" | "target" | "inplace"
    pub out_mode: String,
    /// out_mode == "target" iken seçili hedefin yolu
    pub out_target: String,
    pub local_dir: String,
    pub targets: Vec<Target>,
    pub work_dir: String,
    pub bitrate: u32,
    pub pref_langs: String,
    pub overwrite: bool,
    pub keep_structure: bool,
    pub name_template: String,
    /// İş bitince Windows bildirimi gönder
    pub notify: bool,
    /// UTF-8 olmayan altyazılar için varsayılan kodlama
    pub sub_charset: String,
    /// Arayüz dili: "tr" | "en" | "" (sistem)
    pub lang: String,
    /// Açılışta GitHub'da yeni sürüm denetle
    pub check_updates: bool,
    /// Kullanıcı hazır ayarları (biçimi arayüz belirler)
    pub presets: Vec<Value>,
    /// Eski sürümden kalan alan; yüklenirken hedeflere taşınır
    #[serde(skip_serializing_if = "String::is_empty")]
    pub nas_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            mkvmerge_path: String::new(),
            out_mode: "local".into(),
            out_target: String::new(),
            local_dir: String::new(),
            targets: Vec::new(),
            work_dir: String::new(),
            bitrate: 192,
            pref_langs: "tur,jpn,eng".into(),
            overwrite: false,
            keep_structure: true,
            name_template: "{ad}".into(),
            notify: true,
            sub_charset: "windows-1254".into(),
            lang: String::new(),
            check_updates: true,
            presets: Vec::new(),
            nas_dir: String::new(),
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|d| d.join("settings.json"))
        .map_err(|e| e.to_string())
}

fn migrate(mut s: Settings) -> Settings {
    let nas = s.nas_dir.trim().to_string();
    if !nas.is_empty() {
        if !s.targets.iter().any(|t| t.path == nas) {
            s.targets.push(Target {
                name: "NAS".into(),
                path: nas.clone(),
            });
        }
        if s.out_mode == "nas" {
            s.out_mode = "target".into();
            s.out_target = nas;
        }
        s.nas_dir.clear();
    }
    if s.out_mode == "nas" {
        s.out_mode = "local".into();
    }
    if s.out_mode == "target" && !s.targets.iter().any(|t| t.path == s.out_target) {
        s.out_mode = "local".into();
    }
    if s.name_template.trim().is_empty() {
        s.name_template = "{ad}".into();
    }
    if s.sub_charset.trim().is_empty() {
        s.sub_charset = "windows-1254".into();
    }
    s
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Settings {
    let s = settings_path(&app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    migrate(s)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let p = settings_path(&app)?;
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let txt = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(p, txt).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_text(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| m!("Okunamadı: {e}", "Could not read: {e}"))
}

#[tauri::command]
fn write_text(path: String, text: String) -> Result<(), String> {
    fs::write(&path, text).map_err(|e| m!("Yazılamadı: {e}", "Could not write: {e}"))
}

// ───────────────────────── Kuyruk, geçmiş, log dosyaları ─────────────────────────

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let d = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn logs_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let d = data_dir(app)?.join("logs");
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Log kimliği yalnızca harf, rakam ve tire içerebilir (dosya adı olarak kullanılıyor)
fn safe_id(id: &str) -> Result<&str, String> {
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(id)
    } else {
        Err(m!("Geçersiz kimlik", "Invalid id"))
    }
}

#[tauri::command]
fn save_queue(app: AppHandle, text: String) -> Result<(), String> {
    let p = data_dir(&app)?.join("queue.json");
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_queue(app: AppHandle) -> String {
    data_dir(&app)
        .ok()
        .and_then(|d| fs::read_to_string(d.join("queue.json")).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn load_history(app: AppHandle) -> Vec<Value> {
    let Ok(d) = data_dir(&app) else {
        return vec![];
    };
    let txt = fs::read_to_string(d.join("history.jsonl")).unwrap_or_default();
    let mut v: Vec<Value> = txt
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    v.reverse(); // en yeni en üstte
    v.truncate(1000);
    v
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    let d = data_dir(&app)?;
    let _ = fs::remove_file(d.join("history.jsonl"));
    let _ = fs::remove_dir_all(d.join("logs"));
    Ok(())
}

#[tauri::command]
fn read_job_log(app: AppHandle, id: String) -> Result<String, String> {
    let p = logs_dir(&app)?.join(format!("{}.log", safe_id(&id)?));
    fs::read_to_string(p).map_err(|_| m!("Bu iş için log bulunamadı", "No log found for this job"))
}

fn append_history(app: &AppHandle, entry: &Value) {
    if let Ok(d) = data_dir(app) {
        let p = d.join("history.jsonl");
        // Geçmiş çok büyümesin: 2000 satırı geçince son 1000 satırı tut
        if let Ok(txt) = fs::read_to_string(&p) {
            let lines: Vec<&str> = txt.lines().collect();
            if lines.len() > 2000 {
                let _ = fs::write(&p, lines[lines.len() - 1000..].join("\n") + "\n");
            }
        }
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "{entry}");
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bir işin ayrıntılı kaydı: çalıştırılan komutlar ve araç çıktıları
#[derive(Default)]
struct JobLog(Vec<String>);

impl JobLog {
    fn push(&mut self, s: impl Into<String>) {
        self.0.push(s.into());
    }
    fn cmd(&mut self, c: &Command) {
        let args: Vec<String> = c
            .get_args()
            .map(|a| {
                let a = a.to_string_lossy();
                if a.contains(' ') || a.is_empty() {
                    format!("\"{a}\"")
                } else {
                    a.into_owned()
                }
            })
            .collect();
        self.push(format!(
            "\n$ {} {}",
            c.get_program().to_string_lossy(),
            args.join(" ")
        ));
    }
    fn output(&mut self, code: i32, out: &str) {
        // İlerleme satırlarını (ffmpeg "anahtar=değer", mkvmerge "Progress: %") ayıkla
        let out: Vec<&str> = out
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.is_empty()
                    // mkvmerge ilerlemesi her dilde "...: 37%" biçiminde
                    && !(l.ends_with('%') && l.contains(": ") && l[..l.len() - 1].ends_with(|c: char| c.is_ascii_digit()))
                    && !l.split_once('=').is_some_and(|(k, _)| {
                        !k.is_empty()
                            && k.chars()
                                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                    })
            })
            .collect();
        if !out.is_empty() {
            self.push(out.join("\n"));
        }
        self.push(format!("→ çıkış kodu {code}"));
    }
}

// ───────────────────────── Araçlar (ffmpeg / mkvmerge) ─────────────────────────

fn exe(name: &str) -> String {
    format!("{}{}", name, std::env::consts::EXE_SUFFIX)
}

fn resolve_tool(name: &str, custom: &str, extra_dir: Option<&Path>) -> Option<PathBuf> {
    let file = exe(name);
    let custom = custom.trim().trim_matches('"');
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
        if p.is_dir() && p.join(&file).is_file() {
            return Some(p.join(&file));
        }
        if p.is_dir() && p.join("bin").join(&file).is_file() {
            return Some(p.join("bin").join(&file));
        }
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(d) = extra_dir {
        cands.push(d.join(&file));
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(d) = cur.parent() {
            cands.push(d.join("bin").join(&file));
            cands.push(d.join(&file));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            cands.push(d.join(&file));
        }
    }
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                cands.push(PathBuf::from(&pf).join("MKVToolNix").join(&file));
                cands.push(PathBuf::from(&pf).join("ffmpeg").join("bin").join(&file));
            }
        }
        if let Some(la) = std::env::var_os("LOCALAPPDATA") {
            let winget = PathBuf::from(la).join("Microsoft").join("WinGet");
            cands.push(winget.join("Links").join(&file));
            // winget'in zip paketleri: Packages\Gyan.FFmpeg_...\ffmpeg-7.x-full_build\bin\ffmpeg.exe
            if let Ok(rd) = fs::read_dir(winget.join("Packages")) {
                for pkg in rd.flatten().map(|e| e.path()) {
                    cands.push(pkg.join(&file));
                    cands.push(pkg.join("bin").join(&file));
                    if let Ok(sub) = fs::read_dir(&pkg) {
                        for d in sub.flatten() {
                            cands.push(d.path().join("bin").join(&file));
                        }
                    }
                }
            }
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            cands.push(PathBuf::from(home).join("scoop").join("shims").join(&file));
        }
        cands.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin").join(&file));
        cands.push(PathBuf::from(r"C:\ffmpeg\bin").join(&file));
    }
    cands.into_iter().find(|p| p.is_file())
}

struct Tools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    mkvmerge: PathBuf,
}

#[derive(Serialize)]
struct ToolStatus {
    ffmpeg: Option<String>,
    ffprobe: Option<String>,
    mkvmerge: Option<String>,
}

fn find_tools(s: &Settings) -> ToolStatus {
    let ffmpeg = resolve_tool("ffmpeg", &s.ffmpeg_path, None);
    let ff_dir = ffmpeg
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let ffprobe = resolve_tool("ffprobe", &s.ffprobe_path, ff_dir.as_deref());
    let mkvmerge = resolve_tool("mkvmerge", &s.mkvmerge_path, None);
    let f = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    ToolStatus {
        ffmpeg: f(ffmpeg),
        ffprobe: f(ffprobe),
        mkvmerge: f(mkvmerge),
    }
}

fn tools(s: &Settings) -> Result<Tools, String> {
    let t = find_tools(s);
    let mut missing = vec![];
    if t.ffmpeg.is_none() {
        missing.push("ffmpeg");
    }
    if t.ffprobe.is_none() {
        missing.push("ffprobe");
    }
    if t.mkvmerge.is_none() {
        missing.push("mkvmerge");
    }
    if !missing.is_empty() {
        return Err(m!(
            "Bulunamadı: {} (Ayarlar'dan yol ver)",
            "Not found: {} (set the path in Settings)",
            missing.join(", ")
        ));
    }
    Ok(Tools {
        ffmpeg: t.ffmpeg.unwrap().into(),
        ffprobe: t.ffprobe.unwrap().into(),
        mkvmerge: t.mkvmerge.unwrap().into(),
    })
}

#[tauri::command]
fn check_tools(settings: Settings) -> ToolStatus {
    set_lang(&settings);
    find_tools(&settings)
}

/// Eksik araçları winget ile kur (Windows 10/11). `which`: "ffmpeg" | "mkvtoolnix"
#[tauri::command]
async fn install_tools(settings: Settings, which: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        if !cfg!(windows) {
            return Err(m!(
                "Otomatik kurulum yalnızca Windows'ta",
                "Automatic install is Windows only"
            ));
        }
        let mut log = vec![];
        for w in which {
            let id = match w.as_str() {
                "ffmpeg" => "Gyan.FFmpeg",
                "mkvtoolnix" => "MoritzBunkus.MKVToolNix",
                _ => continue,
            };
            let out = command(Path::new("winget"))
                .args([
                    "install",
                    "-e",
                    "--id",
                    id,
                    "--source",
                    "winget",
                    "--silent",
                    "--accept-source-agreements",
                    "--accept-package-agreements",
                ])
                .stdin(Stdio::null())
                .output()
                .map_err(|e| m!("winget çalıştırılamadı: {e}", "Could not run winget: {e}"))?;
            let txt = String::from_utf8_lossy(&out.stdout).into_owned();
            // 0 = kuruldu; "zaten kurulu" da başarı sayılır
            if !out.status.success() && !txt.to_lowercase().contains("already installed") {
                return Err(m!(
                    "{id} kurulamadı: {}",
                    "{id} could not be installed: {}",
                    tail(&txt, 3)
                ));
            }
            log.push(format!("{id}: OK"));
        }
        Ok(log.join("\n"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn command(p: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(p);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: konsol penceresi açılmasın
    c
}

// ───────────────────────── Dosya listeleme ─────────────────────────

#[derive(Serialize)]
struct InputFile {
    path: String,
    rel: String,
    size: u64,
}

fn is_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn rel_string(p: &Path, base: &Path) -> String {
    p.strip_prefix(base)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<InputFile>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, base, out);
        } else if is_video(&p) && !p.to_string_lossy().ends_with(TMP_EXT) {
            out.push(InputFile {
                rel: rel_string(&p, base),
                size: fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                path: p.to_string_lossy().into_owned(),
            });
        }
    }
}

#[tauri::command]
fn expand_paths(paths: Vec<String>) -> Vec<InputFile> {
    let mut out = Vec::new();
    for s in paths {
        let p = PathBuf::from(&s);
        if p.is_dir() {
            // Klasör bırakıldıysa klasör adı da korunur: "The Mentalist/Season 1/x.mkv"
            let base = p
                .parent()
                .map(|b| b.to_path_buf())
                .unwrap_or_else(|| p.clone());
            walk(&p, &base, &mut out);
        } else if is_video(&p) {
            out.push(InputFile {
                rel: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size: fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                path: s,
            });
        }
    }
    out
}

// ───────────────────────── İnceleme (ffprobe + mkvmerge) ─────────────────────────

/// Video / ses / altyazı parçası. `idx` bu listedeki sıra, `ff_index` ffprobe akış numarası,
/// `tid` mkvmerge parça kimliği.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Stream {
    idx: usize,
    ff_index: u64,
    tid: u64,
    kind: String, // "video" | "audio" | "subtitle"
    codec: String,
    profile: String,
    lang: String,
    title: String,
    is_default: bool,
    forced: bool,
    bitrate: u64,
    // ses
    channels: u32,
    layout: String,
    sample_rate: u32,
    // video
    width: u64,
    height: u64,
    fps: f64,
    bit_depth: u32,
    hdr: String,
    color_primaries: String,
    color_transfer: String,
    color_space: String,
    // altyazı
    text_sub: bool,
    /// Dış dosyadan geliyorsa yolu (gömülü parçalarda boş)
    ext: String,
    /// Dış metin altyazının kodlaması: "utf-8" | "utf-16le" | "utf-16be" | "" (eski kodlama)
    charset: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ProbeInfo {
    duration: f64,
    size: u64,
    /// Okunabilir ad: "Matroska (MKV)", "MP4"
    format: String,
    /// "mkv" | "mp4" | diğer
    container: String,
    bitrate: u64,
    title: String,
    streams: Vec<Stream>,
    attachments: usize,
    chapters: usize,
}

fn probe_raw(ffprobe: &Path, file: &Path) -> Result<Value, String> {
    let out = command(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| m!("ffprobe çalıştırılamadı: {e}", "Could not run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe: {}",
            tail(&String::from_utf8_lossy(&out.stderr), 3)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| {
        m!(
            "ffprobe çıktısı okunamadı: {e}",
            "Could not parse ffprobe output: {e}"
        )
    })
}

fn num(x: &Value) -> u64 {
    x.as_u64()
        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

/// Akış bit hızı: bit_rate yoksa mkvmerge'ün yazdığı BPS etiketine bak
fn stream_bitrate(st: &Value) -> u64 {
    let b = num(&st["bit_rate"]);
    if b > 0 {
        return b;
    }
    let tags = &st["tags"];
    for k in ["BPS", "BPS-eng"] {
        let v = num(&tags[k]);
        if v > 0 {
            return v;
        }
    }
    0
}

fn parse_fps(r: &str) -> f64 {
    match r.split_once('/') {
        Some((a, b)) => {
            let (a, b): (f64, f64) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0));
            if b > 0.0 { a / b } else { 0.0 }
        }
        None => r.parse().unwrap_or(0.0),
    }
}

/// ffprobe çıktısını çözer. mkvmerge kimlikleri (tid) sonradan `attach_tids` ile eklenir.
fn parse_probe(v: &Value) -> ProbeInfo {
    let s = |x: &Value| x.as_str().unwrap_or("").to_string();
    let streams_raw = v["streams"].as_array().cloned().unwrap_or_default();
    let mut streams = Vec::new();
    let mut attachments = 0;
    for st in &streams_raw {
        let ctype = st["codec_type"].as_str().unwrap_or("");
        let disp = &st["disposition"];
        if ctype == "attachment" || (ctype == "video" && disp["attached_pic"].as_i64() == Some(1)) {
            attachments += 1;
            continue;
        }
        if !matches!(ctype, "video" | "audio" | "subtitle") {
            continue;
        }
        let codec = s(&st["codec_name"]);
        let lang = {
            let l = s(&st["tags"]["language"]);
            if l.is_empty() { "und".to_string() } else { l }
        };
        let pix = s(&st["pix_fmt"]);
        let depth = num(&st["bits_per_raw_sample"]) as u32;
        let bit_depth = if ctype != "video" {
            0
        } else if depth > 0 {
            depth
        } else if pix.contains("12") {
            12
        } else if pix.contains("10") {
            10
        } else if pix.is_empty() {
            0
        } else {
            8
        };
        let hdr = if ctype != "video" {
            String::new()
        } else {
            let side = st["side_data_list"].as_array().cloned().unwrap_or_default();
            let dovi = side
                .iter()
                .any(|d| d["side_data_type"].as_str().unwrap_or("").contains("DOVI"));
            if dovi {
                "Dolby Vision".into()
            } else {
                match st["color_transfer"].as_str().unwrap_or("") {
                    "smpte2084" => "HDR10".into(),
                    "arib-std-b67" => "HLG".into(),
                    _ => String::new(),
                }
            }
        };
        streams.push(Stream {
            idx: streams.len(),
            ff_index: st["index"].as_u64().unwrap_or(0),
            tid: 0,
            kind: ctype.to_string(),
            text_sub: ctype == "subtitle" && TEXT_SUBS.contains(&codec.as_str()),
            codec,
            profile: s(&st["profile"]),
            lang,
            title: s(&st["tags"]["title"]),
            is_default: disp["default"].as_i64() == Some(1),
            forced: disp["forced"].as_i64() == Some(1),
            bitrate: stream_bitrate(st),
            channels: st["channels"].as_u64().unwrap_or(0) as u32,
            layout: s(&st["channel_layout"]),
            sample_rate: num(&st["sample_rate"]) as u32,
            width: st["width"].as_u64().unwrap_or(0),
            height: st["height"].as_u64().unwrap_or(0),
            fps: {
                let f = parse_fps(st["avg_frame_rate"].as_str().unwrap_or(""));
                if f > 0.0 {
                    f
                } else {
                    parse_fps(st["r_frame_rate"].as_str().unwrap_or(""))
                }
            },
            bit_depth,
            hdr,
            color_primaries: s(&st["color_primaries"]),
            color_transfer: s(&st["color_transfer"]),
            color_space: s(&st["color_space"]),
            ext: String::new(),
            charset: String::new(),
        });
    }
    let fmt = &v["format"];
    let fname = s(&fmt["format_name"]);
    let (format, container) = if fname.contains("matroska") {
        ("Matroska (MKV)".to_string(), "mkv".to_string())
    } else if fname.contains("mp4") {
        ("MP4".to_string(), "mp4".to_string())
    } else {
        (fname.clone(), fname)
    };
    ProbeInfo {
        duration: fmt["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0.0),
        size: num(&fmt["size"]),
        format,
        container,
        bitrate: num(&fmt["bit_rate"]),
        title: s(&fmt["tags"]["title"]),
        streams,
        attachments,
        chapters: v["chapters"].as_array().map(|c| c.len()).unwrap_or(0),
    }
}

/// mkvmerge -J ile parça kimliklerini al: (id, tür)
fn mkv_identify(mkvmerge: &Path, file: &Path) -> Result<Vec<(u64, String)>, String> {
    let out = command(mkvmerge)
        .arg("-J")
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            m!(
                "mkvmerge çalıştırılamadı: {e}",
                "Could not run mkvmerge: {e}"
            )
        })?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        m!(
            "mkvmerge -J okunamadı: {e}",
            "Could not parse mkvmerge -J: {e}"
        )
    })?;
    if v["container"]["recognized"].as_bool() == Some(false)
        || v["container"]["supported"].as_bool() == Some(false)
    {
        return Err(m!(
            "mkvmerge bu dosya formatını desteklemiyor",
            "mkvmerge does not support this file format"
        ));
    }
    Ok(v["tracks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|t| {
            (
                t["id"].as_u64().unwrap_or(0),
                t["type"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect())
}

/// ffprobe parçalarını mkvmerge kimlikleriyle tür içi sırayla eşleştir
fn attach_tids(info: &mut ProbeInfo, ident: &[(u64, String)]) -> Result<(), String> {
    for kind in ["video", "audio", "subtitle"] {
        let mk = if kind == "subtitle" {
            "subtitles"
        } else {
            kind
        };
        let ids: Vec<u64> = ident
            .iter()
            .filter(|(_, t)| t == mk)
            .map(|(i, _)| *i)
            .collect();
        let ours: Vec<usize> = info
            .streams
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.idx)
            .collect();
        if ids.len() != ours.len() {
            return Err(m!(
                "Parça eşleşmesi kurulamadı ({kind}: ffprobe {}, mkvmerge {})",
                "Could not match tracks ({kind}: ffprobe {}, mkvmerge {})",
                ours.len(),
                ids.len()
            ));
        }
        for (i, idx) in ours.into_iter().enumerate() {
            info.streams[idx].tid = ids[i];
        }
    }
    Ok(())
}

fn full_probe(t: &Tools, file: &Path) -> Result<ProbeInfo, String> {
    let mut info = parse_probe(&probe_raw(&t.ffprobe, file)?);
    let ident = mkv_identify(&t.mkvmerge, file)?;
    attach_tids(&mut info, &ident)?;
    Ok(info)
}

/// Dış altyazı dosyasını incele. `video` verilirse dil/forced bilgisi dosya adından okunur.
fn probe_external(t: &Tools, file: &Path, video: Option<&Path>) -> Result<Vec<Stream>, String> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !file.is_file() {
        return Err(m!(
            "Altyazı dosyası bulunamadı: {}",
            "Subtitle file not found: {}",
            file.display()
        ));
    }
    let mut info = full_probe(t, file).map_err(|e| format!("{name}: {e}"))?;
    let hints = video.map(|v| subs::name_hints(v, file)).unwrap_or_default();
    let mut out: Vec<Stream> = info
        .streams
        .drain(..)
        .filter(|s| s.kind == "subtitle")
        .collect();
    if out.is_empty() {
        return Err(m!(
            "{name}: altyazı parçası bulunamadı",
            "{name}: no subtitle track found"
        ));
    }
    let charset = if out.iter().any(|s| s.text_sub) {
        fs::read(file)
            .map(|b| subs::detect_charset(&b).to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    for s in &mut out {
        s.ext = file.to_string_lossy().into_owned();
        if s.text_sub {
            s.charset = charset.clone();
        }
        if let Some(l) = &hints.lang {
            if s.lang == "und" {
                s.lang = l.clone();
            }
        }
        s.forced |= hints.forced;
        s.is_default = hints.default;
        if s.title.is_empty() {
            s.title = hints.title.clone();
        }
    }
    Ok(out)
}

/// Dış parçaları listenin sonuna ekle, sıra numaralarını düzelt
fn append_streams(info: &mut ProbeInfo, list: Vec<Stream>) {
    for mut s in list {
        s.idx = info.streams.len();
        info.streams.push(s);
    }
}

#[tauri::command]
async fn probe(settings: Settings, path: String) -> Result<ProbeInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        let t = tools(&settings)?;
        let video = Path::new(&path);
        let mut info = full_probe(&t, video)?;
        // Yanındaki altyazılar: bozuk olanı sessizce atla
        for sc in subs::sidecars(video) {
            if let Ok(list) = probe_external(&t, &sc, Some(video)) {
                append_streams(&mut info, list);
            }
        }
        Ok(info)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Elle eklenen altyazı dosyalarını incele
#[tauri::command]
async fn probe_subs(
    settings: Settings,
    video: String,
    paths: Vec<String>,
) -> Result<Vec<Stream>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        let t = tools(&settings)?;
        let mut out = vec![];
        for p in paths {
            out.extend(probe_external(&t, Path::new(&p), Some(Path::new(&video)))?);
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ───────────────────────── Süreç çalıştırma / iptal ─────────────────────────

#[derive(Default)]
struct Runner {
    child: Mutex<Option<Child>>,
    cancel: AtomicBool,
}

impl Runner {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}

/// Süreci çalıştırır; stdout'u \r veya \n ile bölüp satır satır `on_line`'a verir.
/// Dönüş: (çıkış kodu, stdout son satırları + stderr)
fn run_proc(
    r: &Runner,
    mut cmd: Command,
    mut on_line: impl FnMut(&str),
) -> Result<(i32, String), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| m!("Süreç başlatılamadı: {e}", "Could not start process: {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut s);
        s
    });
    *r.child.lock().unwrap() = Some(child);
    if r.cancelled() {
        if let Some(c) = r.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }

    let mut last: Vec<String> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let mut flush = |line: &mut Vec<u8>, last: &mut Vec<String>| {
        if !line.is_empty() {
            let s = String::from_utf8_lossy(line).into_owned();
            on_line(&s);
            last.push(s);
            if last.len() > 15 {
                last.remove(0);
            }
            line.clear();
        }
    };
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                flush(&mut line, &mut last);
            } else {
                line.push(b);
            }
        }
    }
    flush(&mut line, &mut last);

    let status = {
        let mut g = r.child.lock().unwrap();
        let res = match g.as_mut() {
            Some(c) => c.wait().map_err(|e| e.to_string()),
            None => Err(CANCELLED.to_string()),
        };
        *g = None;
        res?
    };
    let err = err_thread.join().unwrap_or_default();
    if r.cancelled() {
        return Err(CANCELLED.into());
    }
    Ok((
        status.code().unwrap_or(-1),
        format!("{}\n{}", last.join("\n"), err),
    ))
}

#[tauri::command]
fn cancel(state: State<'_, Arc<Runner>>) {
    state.cancel.store(true, Ordering::SeqCst);
    if let Some(c) = state.child.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

// ───────────────────────── Çıktı planı ─────────────────────────

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Convert {
    /// "aac" | "ac3" | "eac3" | "opus" | "flac" | "mp3"
    codec: String,
    /// 0 = kaynakla aynı
    channels: u32,
    /// kbps (flac için yok sayılır)
    bitrate: u32,
    /// Ses seviyesini eşitle (EBU R128)
    #[serde(default)]
    normalize: bool,
    /// 5.1 → stereo inerken diyaloğu öne çıkar
    #[serde(default)]
    dialog: bool,
}

/// Video dönüştürme
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct VideoConv {
    /// "h264" | "hevc" | "av1"
    codec: String,
    /// "auto" | "cpu" | "nvenc" | "qsv" | "amf"
    encoder: String,
    /// CRF benzeri kalite değeri (düşük = daha iyi)
    quality: u32,
    /// "fast" | "medium" | "slow"
    speed: String,
    /// 0 = kaynakla aynı
    max_height: u32,
    /// HDR → SDR
    tonemap: bool,
    /// 10-bit → 8-bit
    to8bit: bool,
    /// Görüntüye yakılacak altyazının `Stream::idx` değeri
    burn: Option<usize>,
}

/// Altyazı işlemleri
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
struct SubOpts {
    /// Boş = otomatik (UTF-8 değilse ayarlardaki varsayılan kodlama)
    charset: String,
    /// Bozuk Türkçe karakterleri düzelt (ý þ ð → ı ş ğ)
    fix_tr: bool,
    /// Zaman kaydırma (ms): artı geciktirir, eksi öne alır
    shift: i64,
    /// ASS / VTT → SRT
    to_srt: bool,
}

impl SubOpts {
    /// Metin üzerinde işlem gerektiriyor mu?
    fn text_work(&self) -> bool {
        self.fix_tr || self.shift != 0 || self.to_srt || !self.charset.trim().is_empty()
    }
}

/// Çıktıdaki bir parça. Liste sırası = çıktı sırası.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct OutTrack {
    /// Kaynak parçanın `Stream::idx` değeri
    src: usize,
    /// None = olduğu gibi kopyala; Some = (yalnızca ses) yeniden kodla
    convert: Option<Convert>,
    name: String,
    lang: String,
    default: bool,
    forced: bool,
    /// Yalnızca altyazılarda
    #[serde(default)]
    sub: Option<SubOpts>,
    /// Yalnızca videoda: yeniden kodla
    #[serde(default)]
    video: Option<VideoConv>,
    /// Yalnızca seste: gecikme (ms), artı geciktirir
    #[serde(default)]
    delay: i64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    path: String,
    rel: String,
    /// "local" | "target" | "inplace"
    mode: String,
    target_dir: String,
    /// "mkv" | "mp4"
    container: String,
    /// Uzantısız çıktı adı (şablon arayüzde uygulanır)
    out_name: String,
    /// Dış altyazı dosyaları; parçaları kaynak parçalarının ardından bu sırayla numaralanır
    #[serde(default)]
    externals: Vec<String>,
    tracks: Vec<OutTrack>,
    keep_chapters: bool,
    keep_attachments: bool,
    clear_title: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ConvertResult {
    output: String,
    encoded: usize,
    seconds: f64,
}

#[derive(Serialize, Clone)]
struct Progress<'a> {
    id: &'a str,
    stage: &'a str,
    percent: f64,
}

struct Emitter2<'a> {
    f: &'a dyn Fn(&str, f64),
    last: (String, i32),
}

impl<'a> Emitter2<'a> {
    fn send(&mut self, stage: &str, pct: f64) {
        let p = pct.clamp(0.0, 100.0);
        let key = (stage.to_string(), (p * 2.0) as i32); // ~%0.5 adımlarla gönder
        if key != self.last {
            self.last = key;
            (self.f)(stage, p);
        }
    }
}

fn last_percent(line: &str) -> Option<f64> {
    let i = line.rfind('%')?;
    let digits: String = line[..i]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

fn ff_time_percent(line: &str, dur: f64) -> Option<f64> {
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let us: f64 = v.trim().parse().ok()?;
    if dur > 0.0 {
        Some(us / 1_000_000.0 / dur * 100.0)
    } else {
        None
    }
}

fn clean_lang(l: &str) -> String {
    let l = l.trim();
    let ok = (2..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphabetic());
    if ok {
        l.to_ascii_lowercase()
    } else {
        "und".into()
    }
}

fn with_tmp_ext(p: &Path) -> PathBuf {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    p.with_file_name(format!("{name}.{TMP_EXT}"))
}

/// Windows'ta geçersiz karakterleri temizle
fn sanitize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if t.is_empty() { "cikti".into() } else { t }
}

fn encoder(codec: &str) -> Result<&'static str, String> {
    Ok(match codec {
        "aac" => "aac",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "opus" => "libopus",
        "flac" => "flac",
        "mp3" => "libmp3lame",
        _ => {
            return Err(m!(
                "Desteklenmeyen ses codec'i: {codec}",
                "Unsupported audio codec: {codec}"
            ));
        }
    })
}

fn copy_with_progress(r: &Runner, em: &mut Emitter2, from: &Path, to: &Path) -> Result<(), String> {
    let total = fs::metadata(from).map(|m| m.len()).unwrap_or(0).max(1);
    let mut src = fs::File::open(from).map_err(|e| {
        m!(
            "Geçici dosya açılamadı: {e}",
            "Could not open temporary file: {e}"
        )
    })?;
    let mut dst = fs::File::create(to).map_err(|e| {
        m!(
            "Hedefe yazılamadı ({}): {e}",
            "Could not write to destination ({}): {e}",
            to.display()
        )
    })?;
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut done: u64 = 0;
    loop {
        if r.cancelled() {
            return Err(CANCELLED.into());
        }
        let n = src.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .map_err(|e| m!("Kopyalama hatası: {e}", "Copy error: {e}"))?;
        done += n as u64;
        em.send("copy", done as f64 / total as f64 * 100.0);
    }
    dst.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn work_root(s: &Settings) -> PathBuf {
    if s.work_dir.trim().is_empty() {
        std::env::temp_dir().join("remuxa")
    } else {
        PathBuf::from(s.work_dir.trim())
    }
}

/// Metin altyazı için (dosya uzantısı, ffmpeg biçimi, ffmpeg codec'i)
fn sub_format(codec: &str) -> (&'static str, &'static str, &'static str) {
    match codec {
        "subrip" | "srt" => ("srt", "srt", "copy"),
        "ass" | "ssa" => ("ass", "ass", "copy"),
        "webvtt" => ("vtt", "webvtt", "copy"),
        _ => ("srt", "srt", "srt"),
    }
}

/// Çıktıdaki bir parçanın nereden alınacağı
enum Loc {
    /// Kaynak dosyadaki parça
    Src,
    /// enc.mka içindeki j. parça
    Enc(usize),
    /// Ayrı dosya: dış altyazı ya da işlenmiş (UTF-8) altyazı
    File {
        path: PathBuf,
        tid: u64,
        ff: u64,
        utf8: bool,
    },
}

/// Kaynaktaki metin altyazıları tek geçişte ayrı dosyalara çıkar: (ffprobe no, codec, çıktı)
fn extract_internal(
    r: &Runner,
    em: &mut Emitter2,
    log: &mut JobLog,
    t: &Tools,
    src: &Path,
    items: &[(u64, String, PathBuf)],
    limit_secs: Option<u32>,
    dur: f64,
) -> Result<(), String> {
    let mut c = command(&t.ffmpeg);
    c.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
        "-i",
    ])
    .arg(src);
    for (ff, codec, out) in items {
        let (_, muxer, enc) = sub_format(codec);
        c.arg("-map").arg(format!("0:{ff}")).args(["-c:s", enc]);
        if let Some(l) = limit_secs {
            c.arg("-t").arg(l.to_string());
        }
        c.arg("-f").arg(muxer).arg(out);
    }
    log.cmd(&c);
    let (code, out) = run_proc(r, c, |l| {
        if let Some(p) = ff_time_percent(l, dur) {
            em.send("subs", p);
        }
    })?;
    log.output(code, &out);
    if code != 0 {
        return Err(m!(
            "ffmpeg (altyazı çıkarma) hatası: {}",
            "ffmpeg (subtitle extraction) error: {}",
            tail(&out, 3)
        ));
    }
    Ok(())
}

/// Ham altyazıyı çöz, düzelt, kaydır ve UTF-8 olarak yaz; istenirse SRT'ye çevir. Dönüş: son dosya
#[allow(clippy::too_many_arguments)]
fn process_text(
    r: &Runner,
    log: &mut JobLog,
    t: &Tools,
    raw: &Path,
    codec: &str,
    o: &SubOpts,
    fallback: &str,
    out_stem: &Path,
) -> Result<PathBuf, String> {
    let (ext, _, _) = sub_format(codec);
    let bytes = fs::read(raw).map_err(|e| {
        m!(
            "Altyazı okunamadı ({}): {e}",
            "Could not read subtitle ({}): {e}",
            raw.display()
        )
    })?;
    let mut text = subs::decode(&bytes, &o.charset, fallback)?;
    if o.fix_tr {
        text = subs::fix_turkish(&text);
    }
    text = subs::shift(&text, ext, o.shift);
    let out = out_stem.with_extension(ext);
    fs::write(&out, text.as_bytes())
        .map_err(|e| m!("Altyazı yazılamadı: {e}", "Could not write subtitle: {e}"))?;
    if !o.to_srt || ext == "srt" {
        return Ok(out);
    }
    let srt = out_stem.with_extension("srt");
    let mut c = command(&t.ffmpeg);
    c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&out)
        .args(["-c:s", "srt", "-f", "srt"])
        .arg(&srt);
    log.cmd(&c);
    let (code, o) = run_proc(r, c, |_| {})?;
    log.output(code, &o);
    if code != 0 {
        return Err(m!(
            "ffmpeg (SRT'ye çevirme) hatası: {}",
            "ffmpeg (SRT conversion) error: {}",
            tail(&o, 3)
        ));
    }
    Ok(srt)
}

// ───────────────────────── Video kodlama ─────────────────────────

/// Bu bilgisayarda kullanılabilen kodlayıcılar (codec → aileler: cpu, nvenc, qsv, amf)
#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
struct EncCaps {
    h264: Vec<String>,
    hevc: Vec<String>,
    av1: Vec<String>,
    /// HDR → SDR için gerekli
    zscale: bool,
    /// Metin altyazı yakmak için gerekli
    libass: bool,
}

impl EncCaps {
    fn list(&self, codec: &str) -> &[String] {
        match codec {
            "h264" => &self.h264,
            "hevc" => &self.hevc,
            _ => &self.av1,
        }
    }
}

fn caps_cache() -> &'static Mutex<HashMap<PathBuf, EncCaps>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, EncCaps>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cpu_encoder(codec: &str) -> &'static str {
    match codec {
        "h264" => "libx264",
        "hevc" => "libx265",
        _ => "libsvtav1",
    }
}

/// Donanım kodlayıcı listede olsa bile sürücü/kart yoksa çalışmaz: kısa bir deneme yap
fn hw_works(ffmpeg: &Path, name: &str) -> bool {
    command(ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:r=25:d=0.2",
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn detect_caps(t: &Tools) -> EncCaps {
    if let Some(c) = caps_cache().lock().unwrap().get(&t.ffmpeg) {
        return c.clone();
    }
    let run = |arg: &str| {
        command(&t.ffmpeg)
            .args(["-hide_banner", arg])
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let encs = run("-encoders");
    let filters = run("-filters");
    let has = |list: &str, name: &str| {
        list.lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    let mut c = EncCaps {
        zscale: has(&filters, "zscale"),
        libass: has(&filters, "subtitles"),
        ..Default::default()
    };
    for codec in ["h264", "hevc", "av1"] {
        let mut v = vec![];
        for fam in ["nvenc", "qsv", "amf"] {
            let name = format!("{codec}_{fam}");
            if has(&encs, &name) && hw_works(&t.ffmpeg, &name) {
                v.push(fam.to_string());
            }
        }
        if has(&encs, cpu_encoder(codec)) {
            v.push("cpu".to_string());
        }
        match codec {
            "h264" => c.h264 = v,
            "hevc" => c.hevc = v,
            _ => c.av1 = v,
        }
    }
    caps_cache()
        .lock()
        .unwrap()
        .insert(t.ffmpeg.clone(), c.clone());
    c
}

#[tauri::command]
async fn detect_encoders(settings: Settings) -> Result<EncCaps, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        set_lang(&settings);
        let t = tools(&settings)?;
        Ok(detect_caps(&t))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "auto" ise en hızlı kullanılabilir aile: NVENC > QSV > AMF > işlemci
fn pick_family(caps: &EncCaps, codec: &str, pref: &str) -> Result<String, String> {
    let list = caps.list(codec);
    if pref.is_empty() || pref == "auto" {
        return list.first().cloned().ok_or_else(|| {
            m!(
                "Bu ffmpeg'de {codec} kodlayıcı yok",
                "This ffmpeg has no {codec} encoder"
            )
        });
    }
    if list.iter().any(|f| f == pref) {
        Ok(pref.to_string())
    } else {
        Err(m!(
            "{pref} ile {codec} kodlama bu bilgisayarda kullanılamıyor",
            "{pref} {codec} encoding is not available on this computer"
        ))
    }
}

/// Kodlayıcı argümanları ve piksel biçimi
fn encoder_args(
    codec: &str,
    fam: &str,
    q: u32,
    speed: &str,
    ten_bit: bool,
) -> (Vec<String>, &'static str) {
    let q = q.to_string();
    let sp = |fast: &str, med: &str, slow: &str| -> String {
        match speed {
            "fast" => fast,
            "slow" => slow,
            _ => med,
        }
        .to_string()
    };
    let mut a: Vec<String> = vec!["-c:v".into()];
    match fam {
        "cpu" => {
            a.push(cpu_encoder(codec).into());
            a.extend(["-crf".into(), q]);
            match codec {
                "av1" => a.extend(["-preset".into(), sp("10", "7", "5")]),
                _ => a.extend(["-preset".into(), sp("veryfast", "medium", "slow")]),
            }
            if codec == "hevc" {
                a.extend(["-x265-params".into(), "log-level=error".into()]);
            }
        }
        "nvenc" => {
            a.push(format!("{codec}_nvenc"));
            a.extend([
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                q,
                "-b:v".into(),
                "0".into(),
            ]);
            a.extend(["-preset".into(), sp("p3", "p5", "p7")]);
        }
        "qsv" => {
            a.push(format!("{codec}_qsv"));
            a.extend(["-global_quality".into(), q]);
            a.extend(["-preset".into(), sp("veryfast", "medium", "veryslow")]);
        }
        _ => {
            a.push(format!("{codec}_amf"));
            a.extend([
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                q.clone(),
                "-qp_p".into(),
                q.clone(),
            ]);
            if codec == "h264" {
                a.extend(["-qp_b".into(), q]);
            }
            a.extend(["-quality".into(), sp("speed", "balanced", "quality")]);
        }
    }
    let pix = match (fam, ten_bit) {
        ("cpu", true) => "yuv420p10le",
        ("cpu", false) | ("nvenc", false) => "yuv420p",
        (_, true) => "p010le",
        _ => "nv12",
    };
    (a, pix)
}

/// Görüntüye yakılacak altyazı
enum Burn {
    /// Çalışma klasöründeki metin altyazı dosyasının adı
    Text(String),
    /// Görüntü altyazı: dosya ve ffprobe numarası
    Image(PathBuf, u64),
}

const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

/// Bir video parçasını kodla. `range` = (başlangıç sn, süre sn) deneme kodlaması için.
#[allow(clippy::too_many_arguments)]
fn encode_video(
    r: &Runner,
    em: &mut Emitter2,
    log: &mut JobLog,
    t: &Tools,
    caps: &EncCaps,
    src: &Path,
    st: &Stream,
    vc: &VideoConv,
    burn: Option<&Burn>,
    work: &Path,
    range: Option<(f64, f64)>,
    dur: f64,
    out: &Path,
) -> Result<String, String> {
    let codec = vc.codec.as_str();
    let fam = pick_family(caps, codec, &vc.encoder)?;
    let is_hdr = !st.hdr.is_empty();
    let tonemap = vc.tonemap && is_hdr;
    if tonemap && !caps.zscale {
        return Err(m!(
            "Bu ffmpeg'de zscale filtresi yok, HDR → SDR yapılamıyor (Gyan veya BtbN 'full' sürümünü kullan)",
            "This ffmpeg lacks the zscale filter, HDR → SDR is not possible (use a Gyan or BtbN 'full' build)"
        ));
    }
    if matches!(burn, Some(Burn::Text(_))) && !caps.libass {
        return Err(m!(
            "Bu ffmpeg'de libass yok, metin altyazı yakılamıyor",
            "This ffmpeg lacks libass, text subtitles cannot be burned in"
        ));
    }
    let ten_bit = codec != "h264" && !vc.to8bit && !tonemap && st.bit_depth > 8;
    let (enc_args, pix) = encoder_args(codec, &fam, vc.quality.max(1), &vc.speed, ten_bit);

    // Filtre zinciri: (görüntü altyazı) → ölçek → ton eşleme → (metin altyazı) → piksel biçimi
    let mut graph = String::new();
    let mut label = format!("0:{}", st.ff_index);
    if let Some(Burn::Image(_, ff)) = burn {
        graph.push_str(&format!("[{label}][1:{ff}]overlay=(W-w)/2:(H-h)[ov];"));
        label = "ov".into();
    }
    let mut chain: Vec<String> = vec![];
    if vc.max_height > 0 && st.height > vc.max_height as u64 {
        chain.push(format!("scale=-2:{}:flags=lanczos", vc.max_height));
    }
    if tonemap {
        chain.push(TONEMAP.into());
    }
    if let Some(Burn::Text(name)) = burn {
        let fonts = if work.join("fonts").is_dir() {
            ":fontsdir=fonts"
        } else {
            ""
        };
        chain.push(format!("subtitles=filename={name}{fonts}"));
    }
    chain.push(format!("format={pix}"));
    graph.push_str(&format!("[{label}]{}[vout]", chain.join(",")));

    let mut c = command(&t.ffmpeg);
    c.current_dir(work);
    c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]);
    if let Some((ss, _)) = range {
        c.arg("-ss").arg(format!("{ss:.3}"));
    }
    c.arg("-i").arg(src);
    if let Some(Burn::Image(p, _)) = burn {
        c.arg("-i").arg(p);
    }
    c.arg("-filter_complex").arg(&graph);
    c.args([
        "-map",
        "[vout]",
        "-an",
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
        "-map_chapters",
        "-1",
    ]);
    if let Some((_, len)) = range {
        c.arg("-t").arg(format!("{len:.3}"));
    }
    c.args(&enc_args);
    // Renk bilgisi: SDR'ye çevrildiyse BT.709, değilse kaynağınki
    let known = |v: &str| !v.is_empty() && v != "unknown" && v != "reserved";
    if tonemap {
        c.args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ]);
    } else {
        if known(&st.color_primaries) {
            c.arg("-color_primaries").arg(&st.color_primaries);
        }
        if known(&st.color_transfer) {
            c.arg("-color_trc").arg(&st.color_transfer);
        }
        if known(&st.color_space) {
            c.arg("-colorspace").arg(&st.color_space);
        }
    }
    c.args(["-progress", "pipe:1", "-nostats"]).arg(out);
    let total = range.map(|(_, l)| l).unwrap_or(dur);
    log.cmd(&c);
    let (code, o) = run_proc(r, c, |l| {
        if let Some(p) = ff_time_percent(l, total) {
            em.send("video", p);
        }
    })?;
    log.output(code, &o);
    if code != 0 {
        return Err(m!(
            "ffmpeg (video) hatası: {}",
            "ffmpeg (video) error: {}",
            tail(&o, 3)
        ));
    }
    Ok(fam)
}

/// Kaynaktaki ekleri (fontları) çalışma klasörüne çıkar; ASS yakarken gerekir
fn dump_fonts(r: &Runner, log: &mut JobLog, t: &Tools, src: &Path, work: &Path) {
    let dir = work.join("fonts");
    if dir.is_dir() || fs::create_dir_all(&dir).is_err() {
        return;
    }
    let mut c = command(&t.ffmpeg);
    c.current_dir(&dir)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-dump_attachment:t",
            "",
            "-i",
        ])
        .arg(src)
        .args(["-t", "0", "-f", "null", "-"]);
    log.cmd(&c);
    if let Ok((code, o)) = run_proc(r, c, |_| {}) {
        log.output(code, &o);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SampleResult {
    path: String,
    seconds: f64,
    bytes: u64,
    est_bytes: u64,
    speed: f64,
    encoder: String,
}

/// 30 saniyelik deneme kodlaması: hız ve tahmini boyut
#[tauri::command]
async fn sample_encode(
    settings: Settings,
    path: String,
    stream: usize,
    video: VideoConv,
) -> Result<SampleResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        set_lang(&settings);
        let t = tools(&settings)?;
        let src = PathBuf::from(&path);
        let info = full_probe(&t, &src)?;
        let st = info
            .streams
            .get(stream)
            .filter(|s| s.kind == "video")
            .ok_or_else(|| m!("Video parçası bulunamadı", "Video track not found"))?;
        let caps = detect_caps(&t);
        let dur = info.duration;
        let len = dur.clamp(1.0, 30.0);
        let start = (dur * 0.4).min((dur - len).max(0.0));
        let dir = work_root(&settings).join("ornekler");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let out = dir.join(format!(
            "{}-{}-{}.mkv",
            sanitize_name(&stem),
            video.codec,
            now_ms() % 100000
        ));
        let r = Runner::default();
        let mut log = JobLog::default();
        let mut em = Emitter2 {
            f: &no_progress,
            last: (String::new(), -1),
        };
        let t0 = Instant::now();
        let fam = encode_video(
            &r,
            &mut em,
            &mut log,
            &t,
            &caps,
            &src,
            st,
            &video,
            None,
            &dir,
            Some((start, len)),
            dur,
            &out,
        )?;
        let secs = t0.elapsed().as_secs_f64();
        let bytes = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        Ok(SampleResult {
            path: out.to_string_lossy().into_owned(),
            seconds: secs,
            bytes,
            est_bytes: (bytes as f64 * dur / len) as u64,
            speed: len / secs.max(0.001),
            encoder: fam,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Dosyayı varsayılan uygulamayla aç (deneme kodlamasını izlemek için)
#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Ses parçasını ayrı dosya olarak kaydet (kayıpsız)
#[tauri::command]
async fn export_audio(
    settings: Settings,
    video: String,
    ff_index: u64,
    out: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        set_lang(&settings);
        let t = tools(&settings)?;
        let r = Runner::default();
        let mut c = command(&t.ffmpeg);
        c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&video)
            .arg("-map")
            .arg(format!("0:{ff_index}"))
            .args(["-c", "copy", "-vn", "-sn", "-dn", "-map_metadata", "-1"])
            .arg(&out);
        let (code, o) = run_proc(&r, c, |_| {})?;
        if code != 0 {
            return Err(m!(
                "Dışa aktarma hatası: {}",
                "Export error: {}",
                tail(&o, 3)
            ));
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Planı kaynağa karşı doğrula
fn validate(job: &Job, info: &ProbeInfo) -> Result<(), String> {
    if job.tracks.is_empty() {
        return Err(m!("Çıktıda hiç parça yok", "The output has no tracks"));
    }
    if !matches!(job.container.as_str(), "mkv" | "mp4") {
        return Err(m!(
            "Bilinmeyen kapsayıcı: {}",
            "Unknown container: {}",
            job.container
        ));
    }
    let mut copied = HashSet::new();
    for t in &job.tracks {
        let n = t.src + 1;
        let s = info.streams.get(t.src).ok_or_else(|| {
            m!(
                "Kaynakta #{n} numaralı parça yok",
                "Source has no track #{n}"
            )
        })?;
        match (&t.convert, &t.video) {
            (None, None) => {
                if !copied.insert(t.src) {
                    return Err(m!(
                        "#{n} aynı anda iki kez kopyalanamaz",
                        "#{n} cannot be copied twice"
                    ));
                }
            }
            (Some(c), _) => {
                if s.kind != "audio" {
                    return Err(m!(
                        "#{n} ses değil, ses dönüştürme uygulanamaz",
                        "#{n} is not audio"
                    ));
                }
                encoder(&c.codec)?;
            }
            (None, Some(v)) => {
                if s.kind != "video" {
                    return Err(m!("#{n} video değil", "#{n} is not video"));
                }
                if !matches!(v.codec.as_str(), "h264" | "hevc" | "av1") {
                    return Err(m!(
                        "Bilinmeyen video codec'i: {}",
                        "Unknown video codec: {}",
                        v.codec
                    ));
                }
                if let Some(b) = v.burn {
                    if info.streams.get(b).map(|x| x.kind.as_str()) != Some("subtitle") {
                        return Err(m!(
                            "Yakılacak altyazı bulunamadı (#{})",
                            "Subtitle to burn not found (#{})",
                            b + 1
                        ));
                    }
                }
            }
        }
        if s.kind == "audio" && t.delay != 0 && t.convert.is_none() && job.container == "mp4" {
            return Err(m!(
                "#{n}: MP4'te kopyalanan seste gecikme yapılamaz; sesi dönüştür ya da MKV seç",
                "#{n}: delay on a copied audio track is not possible in MP4; convert it or use MKV"
            ));
        }
        if let Some(o) = &t.sub {
            if s.kind == "subtitle"
                && !s.text_sub
                && (o.fix_tr || o.to_srt || !o.charset.is_empty())
            {
                return Err(m!(
                    "#{} görüntü tabanlı altyazı; sadece zaman kaydırılabilir",
                    "#{} is an image subtitle; it can only be shifted",
                    t.src + 1
                ));
            }
            if s.kind == "subtitle" && !s.text_sub && o.shift != 0 && job.container == "mp4" {
                return Err(m!(
                    "#{} için zaman kaydırma MP4'te yapılamaz",
                    "#{} cannot be shifted in MP4",
                    t.src + 1
                ));
            }
        }
        if job.container == "mp4" && s.kind == "subtitle" && !s.text_sub {
            return Err(m!(
                "#{} ({}) görüntü tabanlı altyazı, MP4'e konamaz",
                "#{} ({}) is an image subtitle and cannot go into MP4",
                t.src + 1,
                s.codec.to_uppercase()
            ));
        }
    }
    Ok(())
}

fn do_convert(
    report: &dyn Fn(&str, f64),
    r: &Runner,
    s: &Settings,
    job: &Job,
    log: &mut JobLog,
) -> Result<ConvertResult, String> {
    let t0 = Instant::now();
    let t = tools(s)?;
    let mut em = Emitter2 {
        f: report,
        last: (String::new(), -1),
    };

    let src = PathBuf::from(&job.path);
    if !src.is_file() {
        return Err(m!("Kaynak dosya bulunamadı", "Source file not found"));
    }

    // ── Hedef yol
    let inplace = job.mode == "inplace";
    // Hedef klasör (genelde ağ) ve yerinde modunda dosya önce çalışma klasöründe hazırlanır
    let staged = inplace || job.mode == "target";
    let file_name = format!("{}.{}", sanitize_name(&job.out_name), job.container);
    let dest = if inplace {
        src.with_file_name(&file_name)
    } else {
        let base = if job.mode == "target" {
            job.target_dir.trim()
        } else {
            s.local_dir.trim()
        };
        if base.is_empty() {
            return Err(m!(
                "Hedef klasör seçilmemiş (Ayarlar)",
                "No destination folder set (Settings)"
            ));
        }
        let mut d = PathBuf::from(base);
        if s.keep_structure {
            let parts: Vec<&str> = job
                .rel
                .split(['/', '\\'])
                .filter(|p| !p.is_empty() && *p != "." && *p != "..")
                .collect();
            for part in &parts[..parts.len().saturating_sub(1)] {
                d.push(sanitize_name(part));
            }
        }
        d.join(&file_name)
    };
    let replaces_src = dest == src;
    log.push(format!("Kaynak: {}", src.display()));
    log.push(format!("Hedef:  {} ({})", dest.display(), job.mode));
    if dest.exists() && !s.overwrite && !replaces_src {
        return Err(m!(
            "Hedefte zaten var: {}",
            "Already exists at destination: {}",
            dest.display()
        ));
    }
    let dest_dir = dest
        .parent()
        .ok_or_else(|| m!("Geçersiz hedef yolu", "Invalid destination path"))?
        .to_path_buf();

    // ── Kaynağı incele ve planı doğrula
    em.send("probe", 0.0);
    let mut info = full_probe(&t, &src)?;
    for e in &job.externals {
        let list = probe_external(&t, Path::new(e), None)?;
        append_streams(&mut info, list);
    }
    log.push(format!(
        "Kaynak parçalar: {}",
        info.streams
            .iter()
            .map(|x| format!(
                "#{} {} {}{} [{}]",
                x.idx + 1,
                x.kind,
                if x.ext.is_empty() {
                    x.codec.clone()
                } else {
                    format!(
                        "{} (dış: {})",
                        x.codec,
                        Path::new(&x.ext)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                },
                if x.channels > 0 {
                    format!(" {}ch", x.channels)
                } else {
                    String::new()
                },
                x.lang
            ))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    log.push(format!(
        "Plan: {}",
        job.tracks
            .iter()
            .map(|o| match (&o.convert, &o.sub) {
                (Some(c), _) =>
                    format!("#{}→{} {}ch {}k", o.src + 1, c.codec, c.channels, c.bitrate),
                (None, Some(so)) if so.text_work() => format!(
                    "#{}[{}{}{}{}]",
                    o.src + 1,
                    if so.charset.is_empty() {
                        "oto"
                    } else {
                        &so.charset
                    },
                    if so.fix_tr { " tr-düzelt" } else { "" },
                    if so.shift != 0 {
                        format!(" {:+}ms", so.shift)
                    } else {
                        String::new()
                    },
                    if so.to_srt { " srt" } else { "" },
                ),
                _ => format!("#{}", o.src + 1),
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    validate(job, &info)?;
    let dur = info.duration;

    let work = work_root(s).join(&job.id);
    fs::create_dir_all(&work).map_err(|e| {
        m!(
            "Çalışma klasörü oluşturulamadı: {e}",
            "Could not create work folder: {e}"
        )
    })?;

    // Dönüştürülecek parçalar: (çıktı sırası, plan parçası)
    let conv: Vec<(usize, &OutTrack)> = job
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, o)| o.convert.is_some())
        .collect();

    let result = (|| -> Result<ConvertResult, String> {
        // ── 1) Ses kodlama (video'ya dokunulmaz)
        let enc = work.join("enc.mka");
        if !conv.is_empty() {
            let mut c = command(&t.ffmpeg);
            c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(&src);
            for (_, o) in &conv {
                c.arg("-map")
                    .arg(format!("0:{}", info.streams[o.src].ff_index));
            }
            c.args([
                "-vn",
                "-sn",
                "-dn",
                "-map_metadata",
                "-1",
                "-map_chapters",
                "-1",
            ]);
            for (j, (_, o)) in conv.iter().enumerate() {
                let cv = o.convert.as_ref().unwrap();
                let enc_name = encoder(&cv.codec)?;
                c.arg(format!("-c:a:{j}")).arg(enc_name);
                let ch = if cv.channels > 0 {
                    cv.channels
                } else {
                    info.streams[o.src].channels
                };
                let src_ch = info.streams[o.src].channels;
                // Diyalog karışımı: orta kanal (konuşma) öne, yanlar ve arkalar kısık
                let dialog = cv.dialog && src_ch > 2 && cv.channels == 2;
                let mut af: Vec<String> = vec![];
                if dialog {
                    af.push("aformat=channel_layouts=5.1".into());
                    af.push("pan=stereo|FL<FC+0.30*FL+0.30*BL|FR<FC+0.30*FR+0.30*BR".into());
                } else if cv.channels > 0 {
                    c.arg(format!("-ac:a:{j}")).arg(cv.channels.to_string());
                }
                if cv.normalize {
                    af.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
                    af.push("aresample=48000".into());
                }
                if o.delay > 0 {
                    af.push(format!("adelay=delays={}:all=1", o.delay));
                } else if o.delay < 0 {
                    af.push(format!(
                        "atrim=start={:.3},asetpts=PTS-STARTPTS",
                        -o.delay as f64 / 1000.0
                    ));
                }
                if cv.codec != "flac" && cv.bitrate > 0 {
                    c.arg(format!("-b:a:{j}"))
                        .arg(format!("{}k", cv.bitrate.clamp(32, 1536)));
                }
                if cv.codec == "opus" && ch > 2 {
                    // libopus "5.1(side)" gibi düzenleri kabul etmiyor, standart düzene çevir
                    let layout = match ch {
                        3 => "3.0",
                        4 => "quad",
                        5 => "5.0",
                        6 => "5.1",
                        7 => "6.1",
                        _ => "7.1",
                    };
                    c.arg(format!("-mapping_family:a:{j}")).arg("1");
                    af.push(format!("aformat=channel_layouts={layout}"));
                }
                if !af.is_empty() {
                    c.arg(format!("-filter:a:{j}")).arg(af.join(","));
                }
            }
            c.args(["-progress", "pipe:1", "-nostats"]).arg(&enc);
            em.send("audio", 0.0);
            log.cmd(&c);
            let (code, out) = run_proc(r, c, |l| {
                if let Some(p) = ff_time_percent(l, dur) {
                    em.send("audio", p);
                }
            })?;
            log.output(code, &out);
            if code != 0 {
                return Err(m!(
                    "ffmpeg (ses) hatası: {}",
                    "ffmpeg (audio) error: {}",
                    tail(&out, 3)
                ));
            }
        }

        // ── 2) Altyazılar: dış metin altyazılar ve işlem istenenler UTF-8 dosya olarak hazırlanır
        let mut locs: Vec<Loc> = Vec::with_capacity(job.tracks.len());
        let mut conv_j = 0;
        let mut internal: Vec<(u64, String, PathBuf)> = vec![];
        let mut texts: Vec<(usize, PathBuf)> = vec![];
        for (p, o) in job.tracks.iter().enumerate() {
            let st = &info.streams[o.src];
            if o.convert.is_some() {
                locs.push(Loc::Enc(conv_j));
                conv_j += 1;
                continue;
            }
            let opts = o.sub.clone().unwrap_or_default();
            if st.kind == "subtitle" && st.text_sub && (!st.ext.is_empty() || opts.text_work()) {
                let raw = if st.ext.is_empty() {
                    let raw = work.join(format!("raw{p}.{}", sub_format(&st.codec).0));
                    internal.push((st.ff_index, st.codec.clone(), raw.clone()));
                    raw
                } else {
                    PathBuf::from(&st.ext)
                };
                texts.push((p, raw));
                locs.push(Loc::Src); // aşağıda işlenmiş dosyayla değişir
            } else if !st.ext.is_empty() {
                locs.push(Loc::File {
                    path: PathBuf::from(&st.ext),
                    tid: st.tid,
                    ff: st.ff_index,
                    utf8: false,
                });
            } else {
                locs.push(Loc::Src);
            }
        }
        if !internal.is_empty() {
            em.send("subs", 0.0);
            extract_internal(r, &mut em, log, &t, &src, &internal, None, dur)?;
        }
        for (p, raw) in &texts {
            let o = &job.tracks[*p];
            let st = &info.streams[o.src];
            let mut opts = o.sub.clone().unwrap_or_default();
            // WebVTT'yi çoğu oynatıcı MKV içinde tanımıyor; MP4'te de mov_text'e zaten çevrilir
            if st.codec == "webvtt" {
                opts.to_srt = true;
            }
            let out = process_text(
                r,
                log,
                &t,
                raw,
                &st.codec,
                &opts,
                &s.sub_charset,
                &work.join(format!("sub{p}")),
            )?;
            log.push(format!("Altyazı #{} hazırlandı (UTF-8)", o.src + 1));
            locs[*p] = Loc::File {
                path: out,
                tid: 0,
                ff: 0,
                utf8: true,
            };
        }

        // ── 3) Video kodlama (en uzun adım)
        let vids: Vec<usize> = job
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, o)| o.video.is_some())
            .map(|(p, _)| p)
            .collect();
        if !vids.is_empty() {
            let caps = detect_caps(&t);
            log.push(format!("Kodlayıcılar: {caps:?}"));
            for p in &vids {
                let o = &job.tracks[*p];
                let vc = o.video.as_ref().unwrap();
                let st = &info.streams[o.src];
                let burn = match vc.burn {
                    None => None,
                    Some(bi) => {
                        let bs = &info.streams[bi];
                        if bs.text_sub {
                            let raw = if bs.ext.is_empty() {
                                let raw =
                                    work.join(format!("burnraw{p}.{}", sub_format(&bs.codec).0));
                                em.send("subs", 0.0);
                                extract_internal(
                                    r,
                                    &mut em,
                                    log,
                                    &t,
                                    &src,
                                    &[(bs.ff_index, bs.codec.clone(), raw.clone())],
                                    None,
                                    dur,
                                )?;
                                raw
                            } else {
                                PathBuf::from(&bs.ext)
                            };
                            let opts = job
                                .tracks
                                .iter()
                                .find(|x| x.src == bi)
                                .and_then(|x| x.sub.clone())
                                .unwrap_or_default();
                            let f = process_text(
                                r,
                                log,
                                &t,
                                &raw,
                                &bs.codec,
                                &opts,
                                &s.sub_charset,
                                &work.join(format!("burn{p}")),
                            )?;
                            if info.attachments > 0 {
                                dump_fonts(r, log, &t, &src, &work);
                            }
                            Some(Burn::Text(
                                f.file_name().unwrap().to_string_lossy().into_owned(),
                            ))
                        } else {
                            let file = if bs.ext.is_empty() {
                                src.clone()
                            } else {
                                PathBuf::from(&bs.ext)
                            };
                            Some(Burn::Image(file, bs.ff_index))
                        }
                    }
                };
                let out = work.join(format!("vid{p}.mkv"));
                em.send("video", 0.0);
                let fam = encode_video(
                    r,
                    &mut em,
                    log,
                    &t,
                    &caps,
                    &src,
                    st,
                    vc,
                    burn.as_ref(),
                    &work,
                    None,
                    dur,
                    &out,
                )?;
                log.push(format!(
                    "Video #{} kodlandı ({} / {fam})",
                    o.src + 1,
                    vc.codec
                ));
                locs[*p] = Loc::File {
                    path: out,
                    tid: 0,
                    ff: 0,
                    utf8: false,
                };
            }
        }

        // ── 4) Birleştirme
        let out_tmp = if staged {
            work.join(format!("out.{}", job.container))
        } else {
            fs::create_dir_all(&dest_dir).map_err(|e| {
                m!(
                    "Çıktı klasörü oluşturulamadı: {e}",
                    "Could not create output folder: {e}"
                )
            })?;
            with_tmp_ext(&dest)
        };
        em.send("mux", 0.0);
        let res = if job.container == "mkv" {
            mux_mkv(
                r, &mut em, log, &t, &src, &enc, &info, job, &locs, &out_tmp, &work,
            )
        } else {
            mux_mp4(r, &mut em, log, &t, &src, &enc, &info, job, &locs, &out_tmp)
        };
        if let Err(e) = res {
            let _ = fs::remove_file(&out_tmp);
            return Err(e);
        }

        // ── 5) Doğrulama
        em.send("verify", 50.0);
        if let Err(e) = verify(&t, &out_tmp, &info, job) {
            let _ = fs::remove_file(&out_tmp);
            return Err(e);
        }
        em.send("verify", 100.0);
        log.push("Doğrulama tamam".to_string());

        // ── 6) Hedefe yerleştir
        if staged {
            fs::create_dir_all(&dest_dir).map_err(|e| {
                m!(
                    "Hedef klasöre erişilemedi ({}): {e}",
                    "Could not access destination folder ({}): {e}",
                    dest_dir.display()
                )
            })?;
            let part = with_tmp_ext(&dest);
            em.send("copy", 0.0);
            if let Err(e) = copy_with_progress(r, &mut em, &out_tmp, &part) {
                let _ = fs::remove_file(&part);
                return Err(e);
            }
            if let Err(e) = fs::rename(&part, &dest) {
                let _ = fs::remove_file(&part);
                return Err(m!(
                    "Hedef dosya yerleştirilemedi: {e}",
                    "Could not place the output file: {e}"
                ));
            }
            // Yerinde modunda ad/uzantı değiştiyse eski dosya kaldırılır (medya sunucusu çift görmesin)
            if inplace && !replaces_src {
                let _ = fs::remove_file(&src);
            }
        } else if let Err(e) = fs::rename(&out_tmp, &dest) {
            let _ = fs::remove_file(&out_tmp);
            return Err(m!(
                "Hedef dosya yerleştirilemedi: {e}",
                "Could not place the output file: {e}"
            ));
        }
        log.push(format!("Yazıldı: {}", dest.display()));

        Ok(ConvertResult {
            output: dest.to_string_lossy().into_owned(),
            encoded: conv.len() + vids.len(),
            seconds: t0.elapsed().as_secs_f64(),
        })
    })();

    let _ = fs::remove_dir_all(&work);
    result
}

fn flag(b: bool) -> u8 {
    if b { 1 } else { 0 }
}

#[allow(clippy::too_many_arguments)]
fn mux_mkv(
    r: &Runner,
    em: &mut Emitter2,
    log: &mut JobLog,
    t: &Tools,
    src: &Path,
    enc: &Path,
    info: &ProbeInfo,
    job: &Job,
    locs: &[Loc],
    out: &Path,
    work: &Path,
) -> Result<(), String> {
    // Girdiler: 0 = kaynak, ardından (varsa) kodlanmış sesler, sonra ayrı dosyalar
    struct Input<'a> {
        path: PathBuf,
        is_src: bool,
        is_enc: bool,
        utf8: bool,
        items: Vec<(u64, &'a OutTrack)>,
    }
    let mut ins: Vec<Input> = vec![Input {
        path: src.to_path_buf(),
        is_src: true,
        is_enc: false,
        utf8: false,
        items: vec![],
    }];
    let enc_i = if locs.iter().any(|l| matches!(l, Loc::Enc(_))) {
        ins.push(Input {
            path: enc.to_path_buf(),
            is_src: false,
            is_enc: true,
            utf8: false,
            items: vec![],
        });
        1
    } else {
        0
    };
    let mut order: Vec<String> = vec![];
    for (o, l) in job.tracks.iter().zip(locs) {
        let (i, tid) = match l {
            Loc::Src => (0, info.streams[o.src].tid),
            Loc::Enc(j) => (enc_i, *j as u64),
            Loc::File {
                path, tid, utf8, ..
            } => {
                let i = match ins
                    .iter()
                    .position(|x| !x.is_src && !x.is_enc && x.path == *path)
                {
                    Some(i) => i,
                    None => {
                        ins.push(Input {
                            path: path.clone(),
                            is_src: false,
                            is_enc: false,
                            utf8: *utf8,
                            items: vec![],
                        });
                        ins.len() - 1
                    }
                };
                (i, *tid)
            }
        };
        ins[i].items.push((tid, o));
        order.push(format!("{i}:{tid}"));
    }

    // Argümanlar JSON seçenek dosyasıyla verilir: her zaman UTF-8 (Türkçe adlar bozulmaz)
    // ve Windows komut satırı uzunluk sınırına takılmaz.
    let mut a: Vec<String> = Vec::new();
    let p = |x: &Path| x.to_string_lossy().into_owned();
    a.push("-o".into());
    a.push(p(out));
    if job.clear_title {
        a.push("--title".into());
        a.push(String::new());
    }
    for inp in &ins {
        if !inp.is_enc {
            // Hangi parçalar alınacak (kodlanmış ses dosyasındakilerin hepsi kullanılır)
            for (kind, keep_opt, none_opt) in [
                ("video", "--video-tracks", "--no-video"),
                ("audio", "--audio-tracks", "--no-audio"),
                ("subtitle", "--subtitle-tracks", "--no-subtitles"),
            ] {
                let ids: Vec<String> = inp
                    .items
                    .iter()
                    .filter(|(_, o)| info.streams[o.src].kind == kind)
                    .map(|(tid, _)| tid.to_string())
                    .collect();
                if ids.is_empty() {
                    a.push(none_opt.into());
                } else {
                    a.push(keep_opt.into());
                    a.push(ids.join(","));
                }
            }
        }
        if inp.is_src {
            if !job.keep_chapters {
                a.push("--no-chapters".into());
            }
            if !job.keep_attachments {
                a.push("--no-attachments".into());
            }
        } else {
            a.push("--no-chapters".into());
            a.push("--no-global-tags".into());
            a.push("--no-attachments".into());
        }
        for (tid, o) in &inp.items {
            a.push("--language".into());
            a.push(format!("{tid}:{}", clean_lang(&o.lang)));
            a.push("--track-name".into());
            a.push(format!("{tid}:{}", o.name));
            a.push("--default-track-flag".into());
            a.push(format!("{tid}:{}", flag(o.default)));
            a.push("--forced-display-flag".into());
            a.push(format!("{tid}:{}", flag(o.forced)));
            if inp.utf8 {
                a.push("--sub-charset".into());
                a.push(format!("{tid}:UTF-8"));
            }
            // Kopyalanan seste gecikme ve görüntü altyazısında kaydırma mkvmerge ile yapılır
            // (kodlanan seste ve işlenmiş metin altyazıda zaten uygulandı)
            let kind = info.streams[o.src].kind.as_str();
            let shift = match kind {
                "audio" => o.delay,
                "subtitle" => o.sub.as_ref().map(|s| s.shift).unwrap_or(0),
                _ => 0,
            };
            if shift != 0 && !inp.utf8 && !inp.is_enc {
                a.push("--sync".into());
                a.push(format!("{tid}:{shift}"));
            }
        }
        a.push(p(&inp.path));
    }
    a.push("--track-order".into());
    a.push(order.join(","));

    let opts = work.join("mkvmerge-opts.json");
    fs::write(&opts, serde_json::to_string(&a).map_err(|e| e.to_string())?).map_err(|e| {
        m!(
            "Seçenek dosyası yazılamadı: {e}",
            "Could not write options file: {e}"
        )
    })?;
    let mut m = command(&t.mkvmerge);
    m.arg(format!("@{}", opts.to_string_lossy()));

    log.push(format!(
        "\n$ {} {}",
        t.mkvmerge.to_string_lossy(),
        serde_json::to_string(&a).unwrap_or_default()
    ));
    let (code, out) = run_proc(r, m, |l| {
        if let Some(p) = last_percent(l) {
            em.send("mux", p);
        }
    })?;
    log.output(code, &out);
    // mkvmerge: 0 = tamam, 1 = uyarılarla tamam, 2 = hata
    if code != 0 && code != 1 {
        return Err(m!(
            "mkvmerge hatası: {}",
            "mkvmerge error: {}",
            tail(&out, 3)
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mux_mp4(
    r: &Runner,
    em: &mut Emitter2,
    log: &mut JobLog,
    t: &Tools,
    src: &Path,
    enc: &Path,
    info: &ProbeInfo,
    job: &Job,
    locs: &[Loc],
    out: &Path,
) -> Result<(), String> {
    // Girdiler: 0 = kaynak, ardından (varsa) kodlanmış sesler, sonra ayrı dosyalar
    let mut inputs: Vec<PathBuf> = vec![src.to_path_buf()];
    let enc_i = if locs.iter().any(|l| matches!(l, Loc::Enc(_))) {
        inputs.push(enc.to_path_buf());
        1
    } else {
        0
    };
    let mut maps: Vec<String> = vec![];
    for (o, l) in job.tracks.iter().zip(locs) {
        maps.push(match l {
            Loc::Src => format!("0:{}", info.streams[o.src].ff_index),
            Loc::Enc(j) => format!("{enc_i}:{j}"),
            Loc::File { path, ff, .. } => {
                let i = match inputs.iter().skip(1 + enc_i).position(|x| x == path) {
                    Some(i) => i + 1 + enc_i,
                    None => {
                        inputs.push(path.clone());
                        inputs.len() - 1
                    }
                };
                format!("{i}:{ff}")
            }
        });
    }

    let mut c = command(&t.ffmpeg);
    c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]);
    for i in &inputs {
        c.arg("-i").arg(i);
    }
    for m in &maps {
        c.arg("-map").arg(m);
    }
    c.args(["-c", "copy"]);
    c.args(["-map_metadata", "0"]);
    c.args(["-map_chapters", if job.keep_chapters { "0" } else { "-1" }]);
    if job.clear_title {
        c.args(["-metadata", "title="]);
    }
    let mut vi = 0;
    for (p, o) in job.tracks.iter().enumerate() {
        let st = &info.streams[o.src];
        if st.kind == "subtitle" {
            c.arg(format!("-c:{p}")).arg("mov_text");
        }
        if st.kind == "video" {
            let out_codec = o
                .video
                .as_ref()
                .map(|v| v.codec.as_str())
                .unwrap_or(&st.codec);
            if out_codec == "hevc" {
                c.arg(format!("-tag:v:{vi}")).arg("hvc1");
            }
            vi += 1;
        }
        c.arg(format!("-metadata:s:{p}"))
            .arg(format!("language={}", clean_lang(&o.lang)));
        c.arg(format!("-metadata:s:{p}"))
            .arg(format!("title={}", o.name));
        // MP4'te parça adı handler_name olarak saklanır
        if !o.name.is_empty() {
            c.arg(format!("-metadata:s:{p}"))
                .arg(format!("handler_name={}", o.name));
        }
        let disp = match (o.default, o.forced) {
            (true, true) => "default+forced",
            (true, false) => "default",
            (false, true) => "forced",
            (false, false) => "0",
        };
        c.arg(format!("-disposition:{p}")).arg(disp);
    }
    c.args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
        .arg("-f")
        .arg("mp4")
        .arg(out);
    let dur = info.duration;
    log.cmd(&c);
    let (code, out) = run_proc(r, c, |l| {
        if let Some(p) = ff_time_percent(l, dur) {
            em.send("mux", p);
        }
    })?;
    log.output(code, &out);
    if code != 0 {
        return Err(m!(
            "ffmpeg (MP4) hatası: {}",
            "ffmpeg (MP4) error: {}",
            tail(&out, 3)
        ));
    }
    Ok(())
}

fn verify(t: &Tools, out: &Path, src_info: &ProbeInfo, job: &Job) -> Result<(), String> {
    let v = parse_probe(&probe_raw(&t.ffprobe, out)?);
    if v.streams.len() != job.tracks.len() {
        return Err(m!(
            "Doğrulama: {} parça bekleniyordu, {} bulundu",
            "Verify: expected {} tracks, found {}",
            job.tracks.len(),
            v.streams.len()
        ));
    }
    for (p, (o, got)) in job.tracks.iter().zip(&v.streams).enumerate() {
        let want = &src_info.streams[o.src];
        if want.kind != got.kind {
            return Err(m!(
                "Doğrulama: {}. parça {} olmalıydı, {} çıktı",
                "Verify: track {} should be {}, got {}",
                p + 1,
                want.kind,
                got.kind
            ));
        }
        if let Some(vc) = &o.video {
            if got.codec != vc.codec {
                return Err(m!(
                    "Doğrulama: {}. parça {} olmalıydı, {} çıktı",
                    "Verify: track {} should be {}, got {}",
                    p + 1,
                    vc.codec,
                    got.codec
                ));
            }
            if vc.max_height > 0 && got.height > vc.max_height as u64 {
                return Err(m!(
                    "Doğrulama: {}. parça en fazla {}p olmalıydı, {}p çıktı",
                    "Verify: track {} should be at most {}p, got {}p",
                    p + 1,
                    vc.max_height,
                    got.height
                ));
            }
        }
        if let Some(cv) = &o.convert {
            if got.codec != cv.codec {
                return Err(m!(
                    "Doğrulama: {}. parça {} olmalıydı, {} çıktı",
                    "Verify: track {} should be {}, got {}",
                    p + 1,
                    cv.codec,
                    got.codec
                ));
            }
            if cv.channels > 0 && got.channels != cv.channels {
                return Err(m!(
                    "Doğrulama: {}. parça {} kanal olmalıydı, {} çıktı",
                    "Verify: track {} should have {} channels, got {}",
                    p + 1,
                    cv.channels,
                    got.channels
                ));
            }
        }
        if got.kind != "video" && got.is_default != o.default {
            return Err(m!(
                "Doğrulama: {}. parçanın varsayılan bayrağı tutmuyor",
                "Verify: default flag of track {} does not match",
                p + 1
            ));
        }
    }
    if src_info.duration > 0.0 && v.duration > 0.0 && (src_info.duration - v.duration).abs() > 5.0 {
        return Err(m!(
            "Doğrulama: süre uyuşmuyor ({:.0}s → {:.0}s)",
            "Verify: duration mismatch ({:.0}s → {:.0}s)",
            src_info.duration,
            v.duration
        ));
    }
    Ok(())
}

// ───────────────────────── Altyazı önizleme / dışa aktarma ─────────────────────────

/// Arayüzden gelen altyazı parçası (Stream'in gereken alanları)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubRef {
    ff_index: u64,
    tid: u64,
    codec: String,
    #[serde(default)]
    ext: String,
    text_sub: bool,
}

fn no_progress(_: &str, _: f64) {}

fn temp_work(s: &Settings, tag: &str) -> Result<PathBuf, String> {
    let d = work_root(s).join(format!("{tag}-{}", now_ms()));
    fs::create_dir_all(&d).map_err(|e| {
        m!(
            "Çalışma klasörü oluşturulamadı: {e}",
            "Could not create work folder: {e}"
        )
    })?;
    Ok(d)
}

/// Metin altyazının ham dosyası: dışsa kendisi, gömülüyse çıkarılır
fn raw_text_sub(
    r: &Runner,
    log: &mut JobLog,
    t: &Tools,
    video: &Path,
    sr: &SubRef,
    work: &Path,
    limit_secs: Option<u32>,
) -> Result<PathBuf, String> {
    if !sr.ext.is_empty() {
        return Ok(PathBuf::from(&sr.ext));
    }
    let raw = work.join(format!("raw.{}", sub_format(&sr.codec).0));
    let mut em = Emitter2 {
        f: &no_progress,
        last: (String::new(), -1),
    };
    extract_internal(
        r,
        &mut em,
        log,
        t,
        video,
        &[(sr.ff_index, sr.codec.clone(), raw.clone())],
        limit_secs,
        0.0,
    )?;
    Ok(raw)
}

/// Altyazının işlenmiş halinden ilk satırlar (karakter kodlamasını kontrol etmek için)
#[tauri::command]
async fn preview_sub(
    settings: Settings,
    video: String,
    stream: SubRef,
    opts: SubOpts,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        if !stream.text_sub {
            return Err(m!(
                "Görüntü tabanlı altyazı önizlenemez",
                "Image subtitles cannot be previewed"
            ));
        }
        let t = tools(&settings)?;
        let r = Runner::default();
        let mut log = JobLog::default();
        let work = temp_work(&settings, "onizleme")?;
        let res = (|| {
            // Gömülü altyazıda dosyanın sadece ilk 20 dakikası okunur
            let raw = raw_text_sub(
                &r,
                &mut log,
                &t,
                Path::new(&video),
                &stream,
                &work,
                Some(1200),
            )?;
            let bytes = fs::read(&raw)
                .map_err(|e| m!("Altyazı okunamadı: {e}", "Could not read subtitle: {e}"))?;
            let mut text = subs::decode(&bytes, &opts.charset, &settings.sub_charset)?;
            if opts.fix_tr {
                text = subs::fix_turkish(&text);
            }
            let fmt = sub_format(&stream.codec).0;
            text = subs::shift(&text, fmt, opts.shift);
            if fmt == "ass" {
                text = subs::ass_preview(&text);
            }
            let lines: Vec<&str> = text.lines().take(150).collect();
            if lines.iter().all(|l| l.trim().is_empty()) {
                return Ok(m!(
                    "(İlk 20 dakikada altyazı yok)",
                    "(No subtitles in the first 20 minutes)"
                ));
            }
            Ok(lines.join("\n"))
        })();
        let _ = fs::remove_dir_all(&work);
        res
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Altyazıyı ayrı dosya olarak kaydet (metin altyazılarda seçili işlemler uygulanır)
#[tauri::command]
async fn export_sub(
    settings: Settings,
    video: String,
    stream: SubRef,
    opts: SubOpts,
    out: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        let t = tools(&settings)?;
        let r = Runner::default();
        let mut log = JobLog::default();
        let video = PathBuf::from(&video);
        let out = PathBuf::from(&out);
        let work = temp_work(&settings, "disa-aktar")?;
        let res = (|| -> Result<(), String> {
            if stream.text_sub {
                let raw = raw_text_sub(&r, &mut log, &t, &video, &stream, &work, None)?;
                let done = process_text(
                    &r,
                    &mut log,
                    &t,
                    &raw,
                    &stream.codec,
                    &opts,
                    &settings.sub_charset,
                    &work.join("cikti"),
                )?;
                fs::copy(&done, &out)
                    .map_err(|e| m!("Kaydedilemedi: {e}", "Could not save: {e}"))?;
                return Ok(());
            }
            if !stream.ext.is_empty() {
                let ext = PathBuf::from(&stream.ext);
                fs::copy(&ext, &out)
                    .map_err(|e| m!("Kaydedilemedi: {e}", "Could not save: {e}"))?;
                // VobSub: .idx ile birlikte .sub da gerekir
                let sub = ext.with_extension("sub");
                if sub.is_file() {
                    fs::copy(&sub, out.with_extension("sub"))
                        .map_err(|e| m!("Kaydedilemedi: {e}", "Could not save: {e}"))?;
                }
                return Ok(());
            }
            let mut c = match stream.codec.as_str() {
                "hdmv_pgs_subtitle" => {
                    let mut c = command(&t.ffmpeg);
                    c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
                        .arg(&video)
                        .arg("-map")
                        .arg(format!("0:{}", stream.ff_index))
                        .args(["-c:s", "copy", "-f", "sup"])
                        .arg(&out);
                    c
                }
                "dvd_subtitle" => {
                    let mkvextract = resolve_tool("mkvextract", "", t.mkvmerge.parent())
                        .ok_or_else(|| {
                            m!(
                                "mkvextract bulunamadı (MKVToolNix ile gelir)",
                                "mkvextract not found (ships with MKVToolNix)"
                            )
                        })?;
                    let mut c = command(&mkvextract);
                    c.arg(&video).arg("tracks").arg(format!(
                        "{}:{}",
                        stream.tid,
                        out.with_extension("idx").to_string_lossy()
                    ));
                    c
                }
                other => {
                    return Err(m!(
                        "{other} altyazısı dışa aktarılamıyor",
                        "{other} subtitles cannot be exported"
                    ));
                }
            };
            c.stdin(Stdio::null());
            log.cmd(&c);
            let (code, o) = run_proc(&r, c, |_| {})?;
            if code != 0 && !(stream.codec == "dvd_subtitle" && code == 1) {
                return Err(m!(
                    "Dışa aktarma hatası: {}",
                    "Export error: {}",
                    tail(&o, 3)
                ));
            }
            Ok(())
        })();
        let _ = fs::remove_dir_all(&work);
        res.map(|_| out.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn convert(
    app: AppHandle,
    state: State<'_, Arc<Runner>>,
    settings: Settings,
    job: Job,
) -> Result<ConvertResult, String> {
    let r = state.inner().clone();
    r.cancel.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        set_lang(&settings);
        safe_id(&job.id)?;
        let id = job.id.clone();
        let report = |stage: &str, percent: f64| {
            let _ = app.emit(
                "job-progress",
                Progress {
                    id: &id,
                    stage,
                    percent,
                },
            );
        };
        let started = now_ms();
        let mut log = JobLog::default();
        log.push(format!(
            "Remuxa {} · iş {}",
            env!("CARGO_PKG_VERSION"),
            job.id
        ));
        let res = do_convert(&report, &r, &settings, &job, &mut log);
        let status = match &res {
            Ok(_) => "ok",
            Err(e) if e == CANCELLED => "cancelled",
            Err(_) => "error",
        };
        if let Err(e) = &res {
            log.push(format!("\nHATA: {e}"));
        }
        if let Ok(d) = logs_dir(&app) {
            let _ = fs::write(d.join(format!("{}.log", job.id)), log.0.join("\n"));
        }
        append_history(
            &app,
            &serde_json::json!({
                "id": job.id,
                "time": started,
                "source": job.path,
                "output": res.as_ref().map(|o| o.output.clone()).unwrap_or_default(),
                "mode": job.mode,
                "status": status,
                "seconds": (now_ms() - started) as f64 / 1000.0,
                "error": res.as_ref().err().cloned().unwrap_or_default(),
                "encoded": res.as_ref().map(|o| o.encoded).unwrap_or(0),
            }),
        );
        res
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::new(Runner::default()))
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            check_tools,
            expand_paths,
            probe,
            convert,
            cancel,
            read_text,
            write_text,
            save_queue,
            load_queue,
            load_history,
            clear_history,
            read_job_log,
            probe_subs,
            preview_sub,
            export_sub,
            detect_encoders,
            sample_encode,
            open_path,
            export_audio,
            install_tools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
