/**
 * Aussprache-Lexikon (lva#943): reine Prüf- und Listenlogik der Oberfläche.
 *
 * Die Regeln spiegeln `managers/tts/lexikon/mod.rs` (`ist_wort`, `ist_aussprache`,
 * `validieren`), damit der Dialog Fehler zeigt, bevor das Backend die ganze Liste
 * mit einer einzigen Meldung abweist. Maßgeblich bleibt das Backend.
 */
import type { LexikonEintrag } from "@/bindings";

export const MAX_WORT = 80;
export const MAX_AUSSPRACHE = 160;
export const MAX_EINTRAEGE = 4096;
/** Obergrenze des Dokuments für KI-Vorschläge in Bytes (UTF-8), wie im Backend. */
export const MAX_DOKUMENT_BYTES = 4 * 1024 * 1024;

const WORT_RE = /^[\p{L}\p{M}\p{N}_]+(?:[-’'][\p{L}\p{M}\p{N}_]+)*$/u;
const AUSSPRACHE_ZEICHEN_RE = /^[\p{L}\p{N}\p{M} \-’'.,]+$/u;
const BUCHSTABE_RE = /\p{L}/u;

export type LexikonBefund =
  | "wort_leer"
  | "wort_zu_lang"
  | "wort_zeichen"
  | "aussprache_leer"
  | "aussprache_zu_lang"
  | "aussprache_zeichen"
  | "doppelt";

const laenge = (s: string) => [...s].length;

export function wortBefund(wort: string): LexikonBefund | null {
  if (wort === "") return "wort_leer";
  if (laenge(wort) > MAX_WORT) return "wort_zu_lang";
  if (!WORT_RE.test(wort) || !BUCHSTABE_RE.test(wort)) return "wort_zeichen";
  return null;
}

export function ausspracheBefund(text: string): LexikonBefund | null {
  if (text === "") return "aussprache_leer";
  if (laenge(text) > MAX_AUSSPRACHE) return "aussprache_zu_lang";
  if (
    text.trim() !== text ||
    !AUSSPRACHE_ZEICHEN_RE.test(text) ||
    !BUCHSTABE_RE.test(text)
  )
    return "aussprache_zeichen";
  return null;
}

export interface ZeilenBefund {
  wort: LexikonBefund | null;
  aussprache: LexikonBefund | null;
}

/**
 * Befunde je Zeile. Doppelt ist ein Wort, wenn es ohne Rücksicht auf Groß/Klein schon
 * vorkommt und mindestens einer der beiden Einträge Groß/Klein nicht beachtet, oder
 * wenn es exakt gleich ist (Regel aus `validieren`). Gemeldet wird die spätere Zeile.
 */
export function befunde(liste: LexikonEintrag[]): ZeilenBefund[] {
  const gesehen = new Map<string, LexikonEintrag[]>();
  return liste.map((e) => {
    let wort = wortBefund(e.wort);
    if (wort === null) {
      const schluessel = e.wort.toLowerCase();
      const andere = gesehen.get(schluessel) ?? [];
      if (
        andere.some(
          (a) => !a.gross_klein || !e.gross_klein || a.wort === e.wort,
        )
      )
        wort = "doppelt";
      gesehen.set(schluessel, [...andere, e]);
    }
    return { wort, aussprache: ausspracheBefund(e.aussprache) };
  });
}

export const hatFehler = (b: ZeilenBefund[]) =>
  b.some((z) => z.wort !== null || z.aussprache !== null);

/** Leerzeilen (beide Felder leer) gelten als nicht angelegt und werden nicht gespeichert. */
export const ohneLeerzeilen = (liste: LexikonEintrag[]) =>
  liste.filter((e) => e.wort.trim() !== "" || e.aussprache.trim() !== "");

/** Gleiche Liste? Reihenfolge zählt, wie im gespeicherten Lexikon. */
export function gleich(a: LexikonEintrag[], b: LexikonEintrag[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (e, i) =>
        e.wort === b[i].wort &&
        e.aussprache === b[i].aussprache &&
        e.gross_klein === b[i].gross_klein,
    )
  );
}

/**
 * Übernimmt Vorschläge: Ein Wort, das es schon gibt (ohne Groß/Klein), wird nicht
 * doppelt angelegt, sondern bleibt beim bestehenden Eintrag.
 */
export function vorschlaegeUebernehmen(
  liste: LexikonEintrag[],
  neu: LexikonEintrag[],
): LexikonEintrag[] {
  const vorhanden = new Set(liste.map((e) => e.wort.toLowerCase()));
  const dazu: LexikonEintrag[] = [];
  for (const e of neu) {
    const k = e.wort.toLowerCase();
    if (vorhanden.has(k)) continue;
    vorhanden.add(k);
    dazu.push({ ...e });
  }
  return [...liste, ...dazu];
}

/** Kürzt den Text auf die Byte-Grenze des Backends, ohne ein Zeichen zu zerschneiden. */
export function dokumentFuerVorschlaege(text: string): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= MAX_DOKUMENT_BYTES) return text;
  return new TextDecoder("utf-8", { fatal: false })
    .decode(bytes.slice(0, MAX_DOKUMENT_BYTES))
    .replace(/�$/u, "");
}

/** Ein markiertes Wort als Vorbelegung, wenn es ein gültiges Lexikon-Wort ist. */
export function wortAusMarkierung(markiert: string | null | undefined): string {
  const w = (markiert ?? "")
    .trim()
    .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
  return wortBefund(w) === null ? w : "";
}
