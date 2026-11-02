//! Altyazı yardımcıları: yan dosya bulma, karakter kodlaması, Türkçe karakter düzeltme, zaman kaydırma.

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};
use std::fs;
use std::path::{Path, PathBuf};

/// Dış altyazı olarak kabul edilen uzantılar
pub const SUB_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sup", "idx"];

pub fn is_sub(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUB_EXT.contains(&e.to_ascii_lowercase().as_str()))
}

/// Videonun yanındaki aynı adlı altyazılar: "Film.srt", "Film.tr.srt", "Film.tur.forced.ass"…
pub fn sidecars(video: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem()) else {
        return vec![];
    };
    let stem = stem.to_string_lossy().to_lowercase();
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_sub(p))
        .filter(|p| {
            let s = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            s == stem || s.starts_with(&format!("{stem}."))
        })
        .collect();
    out.sort();
    out
}

/// Dosya adından okunan bilgiler
#[derive(Debug, Default, PartialEq)]
pub struct Hints {
    pub lang: Option<String>,
    pub forced: bool,
    pub default: bool,
    pub title: String,
}

fn lang_code(tok: &str) -> Option<&'static str> {
    Some(match tok {
        "tr" | "tur" | "turkish" | "türkçe" | "turkce" => "tur",
        "en" | "eng" | "english" | "ingilizce" => "eng",
        "ja" | "jp" | "jpn" | "japanese" => "jpn",
        "de" | "ger" | "deu" | "german" => "ger",
        "fr" | "fre" | "fra" | "french" => "fre",
        "es" | "spa" | "spanish" => "spa",
        "it" | "ita" | "italian" => "ita",
        "ru" | "rus" | "russian" => "rus",
        "pt" | "por" | "portuguese" => "por",
        "ko" | "kor" | "korean" => "kor",
        "zh" | "chi" | "zho" | "chinese" => "chi",
        "ar" | "ara" | "arabic" => "ara",
        "nl" | "dut" | "nld" | "dutch" => "dut",
        "pl" | "pol" | "polish" => "pol",
        "sv" | "swe" | "swedish" => "swe",
        "no" | "nor" | "nb" | "norwegian" => "nor",
        "da" | "dan" | "danish" => "dan",
        "fi" | "fin" | "finnish" => "fin",
        "el" | "gre" | "ell" | "greek" => "gre",
        "he" | "heb" | "hebrew" => "heb",
        "fa" | "per" | "fas" | "persian" => "per",
        "az" | "aze" | "azerbaijani" => "aze",
        "uk" | "ukr" | "ukrainian" => "ukr",
        "hi" | "hin" | "hindi" => "hin",
        _ => return None,
    })
}

pub fn name_hints(video: &Path, sub: &Path) -> Hints {
    let vstem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sstem = sub
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = vstem.len();
    let same_start = sstem.is_char_boundary(n.min(sstem.len()))
        && sstem.len() >= n
        && sstem[..n].to_lowercase() == vstem.to_lowercase();
    let rest = if same_start {
        sstem[n..].to_string()
    } else {
        // Video adıyla eşleşmiyorsa son birkaç parçaya bak
        sstem.rsplit('.').take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(".")
    };
    let mut h = Hints::default();
    let mut extra = vec![];
    for tok in rest.split(['.', '_', ' ', '-']).filter(|t| !t.is_empty()) {
        let l = tok.to_lowercase();
        match l.as_str() {
            "forced" | "zorunlu" => h.forced = true,
            "default" => h.default = true,
            "sdh" | "cc" | "hi" if h.lang.is_some() => extra.push("SDH".to_string()),
            _ => match lang_code(&l) {
                Some(c) if h.lang.is_none() => h.lang = Some(c.into()),
                _ => extra.push(tok.to_string()),
            },
        }
    }
    h.title = extra.join(" ");
    h
}

/// Ham baytın kodlamasını tahmin et: "utf-8" | "utf-16le" | "utf-16be" | "" (UTF-8 değil, eski kodlama)
pub fn detect_charset(b: &[u8]) -> &'static str {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        "utf-8"
    } else if b.starts_with(&[0xFF, 0xFE]) {
        "utf-16le"
    } else if b.starts_with(&[0xFE, 0xFF]) {
        "utf-16be"
    } else if std::str::from_utf8(b).is_ok() {
        "utf-8"
    } else {
        ""
    }
}

/// Baytları metne çevir. `chosen` boşsa otomatik: BOM / geçerli UTF-8, değilse `fallback`.
pub fn decode(b: &[u8], chosen: &str, fallback: &str) -> Result<String, String> {
    let label = if chosen.trim().is_empty() {
        match detect_charset(b) {
            "utf-8" => "utf-8",
            "utf-16le" => "utf-16le",
            "utf-16be" => "utf-16be",
            _ => fallback,
        }
    } else {
        chosen
    };
    let enc = match label {
        "utf-8" => UTF_8,
        "utf-16le" => UTF_16LE,
        "utf-16be" => UTF_16BE,
        l => Encoding::for_label(l.trim().as_bytes())
            .ok_or_else(|| m!("Bilinmeyen karakter kodlaması: {l}", "Unknown character encoding: {l}"))?,
    };
    // decode() BOM varsa ona uyar ve BOM'u atar
    let (text, _, _) = enc.decode(b);
    Ok(text.into_owned())
}

/// Windows-1254 metnin Latin-1/1252 gibi okunmasıyla oluşan bozulmaları düzelt (ý→ı, þ→ş, ð→ğ…)
pub fn fix_turkish(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ý' => 'ı',
            'Ý' => 'İ',
            'þ' => 'ş',
            'Þ' => 'Ş',
            'ð' => 'ğ',
            'Ð' => 'Ğ',
            _ => c,
        })
        .collect()
}

// ───────────────────────── Zaman kaydırma ─────────────────────────

/// "01:02:03,456" / "1:02:03.45" / "02:03.456" biçimlerini çöz: (ms, saat var mı, saat basamak, ayraç, kesir basamak)
fn parse_ts(s: &str) -> Option<(i64, bool, usize, char, usize)> {
    let (main, frac, sep) = match s.rfind([',', '.']) {
        Some(i) => (&s[..i], &s[i + 1..], s[i..].chars().next()?),
        None => return None,
    };
    if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<&str> = main.split(':').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let n: Vec<i64> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
    let (h, m, sec) = if n.len() == 3 { (n[0], n[1], n[2]) } else { (0, n[0], n[1]) };
    let f: i64 = frac.parse().ok()?;
    let ms = f * 10_i64.pow(3 - frac.len() as u32);
    Some((
        ((h * 60 + m) * 60 + sec) * 1000 + ms,
        n.len() == 3,
        parts[0].len(),
        sep,
        frac.len(),
    ))
}

fn fmt_ts(ms: i64, hours: bool, hwidth: usize, sep: char, fdig: usize) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = ms / 60_000 % 60;
    let s = ms / 1000 % 60;
    let f = (ms % 1000) / 10_i64.pow(3 - fdig as u32);
    if hours || h > 0 {
        format!("{h:0hwidth$}:{m:02}:{s:02}{sep}{f:0fdig$}")
    } else {
        format!("{m:02}:{s:02}{sep}{f:0fdig$}")
    }
}

fn shift_ts(tok: &str, delta: i64) -> Option<String> {
    let (ms, hours, hw, sep, fd) = parse_ts(tok)?;
    Some(fmt_ts(ms + delta, hours, hw, sep, fd))
}

/// `fmt`: "srt" | "vtt" | "ass"
pub fn shift(text: &str, fmt: &str, delta: i64) -> String {
    if delta == 0 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix("\r\n") {
            Some(b) => (b, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (line, ""),
            },
        };
        if fmt == "ass" {
            let t = body.trim_start();
            let is_event = t.starts_with("Dialogue:") || t.starts_with("Comment:");
            if is_event {
                // Dialogue: Layer,Start,End,...
                let mut fields: Vec<String> = body.splitn(4, ',').map(str::to_string).collect();
                if fields.len() == 4 {
                    for i in [1, 2] {
                        if let Some(s) = shift_ts(fields[i].trim(), delta) {
                            fields[i] = s;
                        }
                    }
                    out.push_str(&fields.join(","));
                    out.push_str(nl);
                    continue;
                }
            }
            out.push_str(line);
        } else if body.contains("-->") {
            // Zaman satırı: "00:00:01,000 --> 00:00:02,000 [vtt ayarları]"
            let shifted: Vec<String> = body
                .split(' ')
                .map(|tok| shift_ts(tok, delta).unwrap_or_else(|| tok.to_string()))
                .collect();
            out.push_str(&shifted.join(" "));
            out.push_str(nl);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// ASS metnini okunur hale getir: "0:00:01.00 → 0:00:02.00  Metin" (stil etiketleri atılır)
pub fn ass_preview(text: &str) -> String {
    let mut out = vec![];
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("Dialogue:") else {
            continue;
        };
        let f: Vec<&str> = rest.splitn(10, ',').collect();
        if f.len() < 10 {
            continue;
        }
        let mut body = String::new();
        let mut in_tag = false;
        for c in f[9].chars() {
            match c {
                '{' => in_tag = true,
                '}' => in_tag = false,
                _ if !in_tag => body.push(c),
                _ => {}
            }
        }
        let body = body.replace("\\N", " / ").replace("\\n", " / ").replace("\\h", " ");
        out.push(format!("{} → {}  {}", f[1].trim(), f[2].trim(), body.trim()));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_shift() {
        let s = "1\r\n00:00:01,500 --> 00:00:03,000\r\nMerhaba\r\n";
        assert_eq!(
            shift(s, "srt", 2500),
            "1\r\n00:00:04,000 --> 00:00:05,500\r\nMerhaba\r\n"
        );
        // Negatif kaydırma sıfırda kırpılır
        assert_eq!(
            shift(s, "srt", -2000),
            "1\r\n00:00:00,000 --> 00:00:01,000\r\nMerhaba\r\n"
        );
    }

    #[test]
    fn vtt_shift() {
        let s = "WEBVTT\n\n00:01.000 --> 00:02.500 align:start\nHi\n";
        assert_eq!(
            shift(s, "vtt", 1000),
            "WEBVTT\n\n00:02.000 --> 00:03.500 align:start\nHi\n"
        );
    }

    #[test]
    fn ass_shift() {
        let s = "[Events]\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Selam, dünya\n";
        assert_eq!(
            shift(s, "ass", -500),
            "[Events]\nDialogue: 0,0:00:00.50,0:00:02.00,Default,,0,0,0,,Selam, dünya\n"
        );
    }

    #[test]
    fn hour_rollover() {
        assert_eq!(shift_ts("00:59:59,900", 200).unwrap(), "01:00:00,100");
    }

    #[test]
    fn charset() {
        // "ığş" Windows-1254 ile
        let b = [0xFD, 0xF0, 0xFE];
        assert_eq!(detect_charset(&b), "");
        assert_eq!(decode(&b, "", "windows-1254").unwrap(), "ığş");
        // Aynı baytlar 1252 ile okunursa bozulur, düzeltme geri getirir
        let bad = decode(&b, "windows-1252", "").unwrap();
        assert_eq!(bad, "ýðþ");
        assert_eq!(fix_turkish(&bad), "ığş");
        assert_eq!(detect_charset("çok güzel".as_bytes()), "utf-8");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("şey".as_bytes());
        assert_eq!(decode(&bom, "", "windows-1254").unwrap(), "şey");
    }

    #[test]
    fn ass_view() {
        let s = "[Events]\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Selam{\\i0}, dünya\\Nikinci\n";
        assert_eq!(ass_preview(s), "0:00:01.00 → 0:00:02.50  Selam, dünya / ikinci");
    }

    #[test]
    fn hints() {
        let v = Path::new("/x/The.Movie.2020.mkv");
        let h = name_hints(v, Path::new("/x/The.Movie.2020.tr.forced.srt"));
        assert_eq!(h.lang.as_deref(), Some("tur"));
        assert!(h.forced);
        let h = name_hints(v, Path::new("/x/The.Movie.2020.en.sdh.srt"));
        assert_eq!(h.lang.as_deref(), Some("eng"));
        assert_eq!(h.title, "SDH");
        let h = name_hints(v, Path::new("/x/The.Movie.2020.srt"));
        assert_eq!(h, Hints::default());
        let h = name_hints(v, Path::new("/x/The.Movie.2020.Turkish.Netflix.srt"));
        assert_eq!(h.lang.as_deref(), Some("tur"));
        assert_eq!(h.title, "Netflix");
    }
}
