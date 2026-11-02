import { EN } from "./i18n-en";

export type Lang = "tr" | "en";

/** Etkin arayüz dili. Ayarlarda "" ise sistem dili kullanılır. */
export const i18n = $state<{ lang: Lang }>({ lang: "tr" });

export function resolveLang(pref: string): Lang {
  if (pref === "tr" || pref === "en") return pref;
  return (navigator.language || "tr").toLowerCase().startsWith("tr") ? "tr" : "en";
}

/** Türkçe metni etkin dile çevirir; {ad} biçimindeki yer tutucuları doldurur. */
export function t(s: string, p?: Record<string, string | number>): string {
  let out = i18n.lang === "en" ? (EN[s] ?? s) : s;
  // "tekil|çoğul" biçimi: n === 1 ise ilk hali
  if (out.includes("|") && p && typeof p.n === "number") out = out.split("|")[p.n === 1 ? 0 : 1];
  if (p) for (const k in p) out = out.replaceAll(`{${k}}`, String(p[k]));
  return out;
}
