/**
 * Vorlesestimme je Buchsprache (lva#352, Patrick 06.11.2026: „für deutsche
 * Bücher eine Stimme, für englische eine andere – nicht immer umstellen“).
 *
 * Zwei Ebenen, beide unter `lva.ui.` und im Kern gespiegelt (`tts.voice.*`
 * reist wie Lesezeichen in der Sync-Sammlung `bookmark` zu allen Geräten):
 * - `tts.voice.lang.<sprache>`: Standardstimme einer Sprache. Eine Wahl im
 *   Reader ändert nur die Stimme der Sprache des offenen Texts.
 * - `tts.voice.book.<buch>.<sprache>`: Stimme nur für dieses Buch in dieser
 *   Sprache (jede Sprachfassung eines Buchs behält so ihre eigene).
 *
 * Wert: JSON {@link VoicePref}. Stimmkennungen sind je Plattform verschieden;
 * `phonon` (`<sprache>:<name>`, z. B. `de:Anton`) ist die gemeinsame Form,
 * die Windows, Mac und iPhone gleich verstehen. `desktop`/`ios` gelten nur
 * auf der eigenen Plattform; wer schreibt, verwirft die Kennung der anderen
 * Plattform, außer beide meinen dieselbe Phonon-Stimme.
 */
import { getLocal, setLocal } from "../library/progress";

export const VOICE_PREF_PREFIX = "tts.voice.";
const LANG = `${VOICE_PREF_PREFIX}lang.`;
const BOOK = `${VOICE_PREF_PREFIX}book.`;
/** Fensterereignis nach jeder Änderung (auch von anderen Ansichten). */
export const VOICE_PREFS_EVENT = "lv-voice-prefs-changed";

export interface VoicePref {
  /** Wert der Desktop-Stimmenauswahl (`provider:phonon:Anton`, `piper:<id>` …). */
  desktop?: string;
  /** Kennung der iPhone-Stimme (`phonon:de:Anton`, AVSpeech-Kennung). */
  ios?: string;
  /** Plattformneutrale Phonon-Stimme `<sprache>:<name>`. */
  phonon?: string;
  /** Zuletzt geändert (Unix-ms); der neuere Stand gewinnt im Abgleich. */
  updated: number;
}

const LANG_RE = /^[a-z]{2,3}$/;

/** Sprachcode für die Schlüssel: `de`, `en` … oder `null`. */
export function prefLanguage(lang: string | null | undefined): string | null {
  const code = (lang ?? "").trim().toLowerCase().split(/[-_]/)[0] ?? "";
  return LANG_RE.test(code) ? code : null;
}

/** Buch- oder Seitenkennung als Schlüsselteil (keine Punkte, kein Leerraum). */
function ownerPart(owner: string): string | null {
  const clean = owner.trim();
  return clean && /^[A-Za-z0-9_-]+$/.test(clean) ? clean : null;
}

export function langVoiceKey(lang: string): string | null {
  const l = prefLanguage(lang);
  return l ? `${LANG}${l}` : null;
}

export function bookVoiceKey(owner: string, lang: string): string | null {
  const o = ownerPart(owner);
  const l = prefLanguage(lang);
  return o && l ? `${BOOK}${o}.${l}` : null;
}

export function parsePref(raw: string | null): VoicePref | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<VoicePref> | null;
    if (!v || typeof v !== "object") return null;
    const str = (x: unknown) =>
      typeof x === "string" && x.length > 0 && x.length <= 512 ? x : undefined;
    const pref: VoicePref = {
      desktop: str(v.desktop),
      ios: str(v.ios),
      phonon: str(v.phonon),
      updated: typeof v.updated === "number" ? v.updated : 0,
    };
    return pref.desktop || pref.ios || pref.phonon ? pref : null;
  } catch {
    return null;
  }
}

export function readPref(key: string | null): VoicePref | null {
  return key ? parsePref(getLocal(key)) : null;
}

function notify() {
  if (typeof window !== "undefined")
    window.dispatchEvent(new CustomEvent(VOICE_PREFS_EVENT));
}

/**
 * Desktop-Wahl speichern. Die iPhone-Kennung bleibt nur stehen, wenn sie
 * dieselbe Phonon-Stimme meint; sonst würde das iPhone eine alte Wahl lesen.
 */
export function writeDesktopPref(
  key: string | null,
  desktop: string,
  phonon: string | null,
  now = Date.now(),
): void {
  if (!key || !desktop) return;
  const old = readPref(key);
  if (old?.desktop === desktop && (old.phonon ?? null) === phonon) return;
  const pref: VoicePref = { desktop, updated: now };
  if (phonon) {
    pref.phonon = phonon;
    if (old?.ios && old.phonon === phonon) pref.ios = old.ios;
  }
  setLocal(key, JSON.stringify(pref));
  notify();
}

/**
 * Eintrag aufheben. Ein leerer Stand (nur `updated`) statt Löschen: er reist
 * im Abgleich wie jede Wahl, und der neuere Stand gewinnt auf allen Geräten.
 */
export function clearPref(key: string | null, now = Date.now()): void {
  if (!key || !readPref(key)) return;
  setLocal(key, JSON.stringify({ updated: now }));
  notify();
}

/** Wo die wirksame Stimme herkommt. */
export type VoicePrefSource = "book" | "lang";

/**
 * Die gemerkte Stimme für Buch und Sprache: zuerst die des Buchs, dann die
 * der Sprache. `toValue` übersetzt einen Eintrag in einen Wert der
 * Stimmenauswahl (`null`: auf diesem Gerät nicht vorhanden – dann gilt die
 * nächste Ebene).
 */
export function resolveVoicePref(
  owner: string | null,
  lang: string | null,
  toValue: (pref: VoicePref) => string | null,
): { value: string; source: VoicePrefSource } | null {
  if (!lang) return null;
  const book = owner ? readPref(bookVoiceKey(owner, lang)) : null;
  const fromBook = book ? toValue(book) : null;
  if (fromBook) return { value: fromBook, source: "book" };
  const fromLang = (() => {
    const p = readPref(langVoiceKey(lang));
    return p ? toValue(p) : null;
  })();
  return fromLang ? { value: fromLang, source: "lang" } : null;
}

/** Hat das Buch für diese Sprache eine eigene Stimme? */
export function hasBookPref(
  owner: string | null,
  lang: string | null,
): boolean {
  return !!(owner && lang && readPref(bookVoiceKey(owner, lang)));
}

/** Phonon-Name ohne Sprachvorsatz: `en:Zoey` → `Zoey`. */
export function phononName(voiceId: string): string {
  return voiceId.replace(/^[a-z]{2,3}:/, "");
}
