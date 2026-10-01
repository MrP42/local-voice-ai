import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type LanguageInfo } from "@/bindings";
import type { SelectOption } from "../../../ui/Select";

/**
 * Die Sprachen der Auswahllisten (Sprache korrigieren, Neu-Transkription,
 * Übersetzen, Ausgabesprache). Dieselben Codes kennt das Backend
 * (`meetings::language::LANGUAGES`); die Namen liefert der Browser in der
 * Sprache der App (`Intl.DisplayNames`), so braucht es keinen Schlüssel je Sprache.
 */
export const LANGUAGE_CODES = [
  "de",
  "en",
  "fr",
  "es",
  "it",
  "pt",
  "nl",
  "pl",
  "cs",
  "sk",
  "sl",
  "hr",
  "hu",
  "ro",
  "bg",
  "el",
  "da",
  "sv",
  "no",
  "fi",
  "et",
  "lv",
  "lt",
  "mt",
  "uk",
  "ru",
  "tr",
  "ar",
  "he",
  "fa",
  "hi",
  "zh",
  "ja",
  "ko",
  "vi",
  "th",
  "id",
] as const;

/** `en-US` -> `en`, `EN` -> `en`; `null` für `auto`, Leeres und Unsinn. */
export const baseLanguage = (tag: string | null | undefined): string | null => {
  const base = (tag ?? "").trim().split(/[-_]/)[0]?.toLowerCase() ?? "";
  return /^[a-z]{2,3}$/.test(base) && base !== "auto" ? base : null;
};

/** Der Name einer Sprache in der Sprache der Oberfläche; unbekannt: der Code. */
export const languageName = (code: string, locale: string): string => {
  try {
    const name = new Intl.DisplayNames([locale], { type: "language" }).of(code);
    if (!name || name === code) return code;
    return name.charAt(0).toLocaleUpperCase(locale) + name.slice(1);
  } catch {
    return code;
  }
};

/** Auswahlliste: Deutsch und Englisch vorn, der Rest nach Namen. */
export const languageOptions = (locale: string): SelectOption[] => {
  const options = LANGUAGE_CODES.map((code) => ({
    value: code,
    label: languageName(code, locale),
  }));
  const head = options.filter((o) => o.value === "de" || o.value === "en");
  const rest = options
    .filter((o) => o.value !== "de" && o.value !== "en")
    .sort((a, b) => a.label.localeCompare(b.label, locale));
  return [...head, ...rest];
};

/** Die Sprache der App als Sprachcode ("de-DE" -> "de"). */
export const appLanguage = (i18nLanguage: string): string =>
  baseLanguage(i18nLanguage) ?? "de";

/** Wert für "wie das Transkript" in den Auswahllisten (Select kennt keinen leeren Wert). */
export const SAME_AS_TRANSCRIPT = "__same__";

// ---------------------------------------------------------------------------
// Letzte Wahl der Ausgabesprache
// ---------------------------------------------------------------------------

const OUTPUT_KEY = "lva.meetings.outputLanguage";

/**
 * Die zuletzt gewählte Ausgabesprache für Protokoll und KI-Notizen (`null`: noch nie
 * gewählt, dann gilt die Sprache der App). Nur ein bewusstes Wählen schreibt: ein
 * Standardwert, der sich beim ersten Anzeigen festschriebe, folgte einem Wechsel der
 * App-Sprache nie mehr.
 */
export const storedOutputLanguage = (): string | null => {
  try {
    const stored = window.localStorage.getItem(OUTPUT_KEY);
    if (stored === SAME_AS_TRANSCRIPT) return stored;
    return baseLanguage(stored);
  } catch {
    return null;
  }
};

export const storeOutputLanguage = (value: string) => {
  try {
    window.localStorage.setItem(OUTPUT_KEY, value);
  } catch {
    /* nicht merken ist verkraftbar */
  }
};

/**
 * Die Ausgabesprache, die ein Lauf bekommt, der keinen Dialog hat: die letzte Wahl,
 * sonst die Sprache der App. `null` heißt "wie das Transkript" (letzte Wahl).
 */
export const defaultOutputLanguage = (i18nLanguage: string): string | null => {
  const stored = storedOutputLanguage();
  if (stored === SAME_AS_TRANSCRIPT) return null;
  return stored ?? appLanguage(i18nLanguage);
};

// ---------------------------------------------------------------------------
// Sprache einer Besprechung
// ---------------------------------------------------------------------------

/**
 * Sprache, Herkunft und Modell der Transkription (Chip im Kopf). Lädt beim Wechsel der
 * Besprechung, bei jeder Änderung von `refreshKey` (Status, Fassung, Neu-Transkription)
 * und auf Anforderung; eine späte Antwort einer alten Anfrage überschreibt nie eine neuere.
 */
export function useLanguageInfo(meetingId: string, refreshKey: string) {
  const [info, setInfo] = useState<LanguageInfo | null>(null);
  const run = useRef(0);

  const reload = useCallback(async () => {
    const seq = ++run.current;
    const result = await commands.meetingsLanguageInfo(meetingId);
    if (seq !== run.current) return;
    setInfo(result.status === "ok" ? result.data : null);
  }, [meetingId]);

  useEffect(() => {
    void reload();
    return () => {
      run.current += 1;
    };
  }, [reload, refreshKey]);

  useEffect(() => {
    setInfo(null);
  }, [meetingId]);

  return { info, setInfo, reload };
}
