import { TTS_TARGET_LANGS } from "@/lib/constants/languages";

/**
 * Sprache eines KI-Dokuments (lva#217), z. B. ein deutsches Kurzbuch zu
 * einem englischen Buch. Wahl wie bei der Übersetzung; "" heißt: wie das
 * Buch. Vorbelegt mit der zuletzt gewählten Sprache, sonst der App-Sprache.
 */
const KEY = "lva.kiDocs.targetLang";

export const KI_LANGS = TTS_TARGET_LANGS.map((l) => ({
  code: l.code,
  label: l.label,
}));

export const isKiLang = (code: string) => KI_LANGS.some((l) => l.code === code);

/** Anzeigename („Deutsch“, „English“ …) zu einem Sprachcode. */
export const kiLangLabel = (code: string | null | undefined) =>
  (code && KI_LANGS.find((l) => l.code === code)?.label) || code || "";

export function defaultKiLang(uiLang: string): string {
  try {
    const v = localStorage.getItem(KEY);
    if (v !== null && (v === "" || isKiLang(v))) return v;
  } catch {
    /* ohne Speicher: App-Sprache */
  }
  const base = uiLang.slice(0, 2).toLowerCase();
  return isKiLang(base) ? base : "";
}

export function rememberKiLang(code: string) {
  try {
    localStorage.setItem(KEY, code);
  } catch {
    /* nur Bequemlichkeit */
  }
}
