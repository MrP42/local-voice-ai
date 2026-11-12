/**
 * Rohfassung eingefügter Texte (lva#530 I1, Konzept §4.1).
 *
 * Vor dem Bereinigen wird der eingefügte Rohtext in der Zustandsdatei der
 * Seite (`state.json`, Feld `einfuegungen`) gesichert. So bleibt er nach einem
 * Neustart verfügbar und lässt sich zurückholen, auch wenn Strg+Z längst
 * nichts mehr weiß. Alte Zustände ohne das Feld bleiben gültig. Reine Logik,
 * kein Zugriff auf Dateien; Inhalte gehen nie ins Log.
 */
import type { PasteRule } from "./pasteCleanup.ts";

/** Höchstens so viele Einfügungen je Seite (die ältesten fallen zuerst). */
export const PASTE_LOG_MAX_ENTRIES = 20;
/** Größter Rohtext (UTF-16-Einheiten), der gesichert und bereinigt wird. */
export const PASTE_RAW_MAX_CHARS = 2 * 1024 * 1024;
/** Obergrenze für Roh- plus bereinigte Fassungen aller Einträge zusammen
 *  (state.json wird bei jeder Änderung ganz geschrieben und synchronisiert). */
export const PASTE_LOG_MAX_TOTAL_CHARS = 16 * 1024 * 1024;

export interface PasteLogRule {
  rule: PasteRule;
  count: number;
}

export interface PasteLogEntry {
  /** Zeitpunkt des Einfügens, ISO 8601. */
  zeit: string;
  /** SHA-256 (hex) des Rohtexts mit vereinheitlichten Zeilenenden. */
  sha256: string;
  /** Rohtext; fehlt, wenn er zu groß war. */
  roh?: string;
  /** Bereinigter Text, wie er eingefügt wurde; fehlt, wenn zu groß. */
  bereinigt?: string;
  /** Änderungen je Regel. */
  regeln: PasteLogRule[];
  /** Zu groß für Sicherung und Bereinigung: unverändert eingefügt. */
  zuGross?: true;
  /** Länge des Rohtexts (nur bei `zuGross`). */
  zeichen?: number;
}

const RULES: readonly PasteRule[] = [
  "control",
  "pageNumbers",
  "repeated",
  "webNoise",
  "fragments",
  "spaces",
  "blankLines",
];

const normalizeNewlines = (text: string) => text.replace(/\r\n?/g, "\n");

/** SHA-256 als Hex-Text; leer, wenn die Umgebung kein WebCrypto bietet. */
export async function sha256Hex(text: string): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) return "";
  const digest = await subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

/** Eintrag für eine bereinigte Einfügung. */
export async function makePasteEntry(
  raw: string,
  cleaned: string,
  rules: readonly PasteLogRule[],
  now: Date = new Date(),
): Promise<PasteLogEntry> {
  const roh = normalizeNewlines(raw);
  return {
    zeit: now.toISOString(),
    sha256: await sha256Hex(roh),
    roh,
    bereinigt: cleaned,
    regeln: rules.map(({ rule, count }) => ({ rule, count })),
  };
}

/** Eintrag für einen zu großen Text: nur Prüfsumme und Länge, keine Rohfassung. */
export async function makeOversizedEntry(
  raw: string,
  now: Date = new Date(),
): Promise<PasteLogEntry> {
  return {
    zeit: now.toISOString(),
    sha256: await sha256Hex(normalizeNewlines(raw)),
    regeln: [],
    zuGross: true,
    zeichen: raw.length,
  };
}

const sizeOf = (entry: PasteLogEntry) =>
  (entry.roh?.length ?? 0) + (entry.bereinigt?.length ?? 0);

/** Neuen Eintrag anhängen (neueste zuletzt); Grenzen für Anzahl und Umfang einhalten. */
export function appendPasteEntry(
  log: readonly PasteLogEntry[],
  entry: PasteLogEntry,
): PasteLogEntry[] {
  const next = [...log, entry].slice(-PASTE_LOG_MAX_ENTRIES);
  let total = next.reduce((sum, e) => sum + sizeOf(e), 0);
  while (next.length > 1 && total > PASTE_LOG_MAX_TOTAL_CHARS) {
    total -= sizeOf(next[0]);
    next.shift();
  }
  return next;
}

const isRule = (value: unknown): value is PasteRule =>
  typeof value === "string" && (RULES as readonly string[]).includes(value);

/** Eintrag aus gespeichertem JSON prüfen; Unbrauchbares verwerfen. */
function parseEntry(value: unknown): PasteLogEntry | null {
  if (typeof value !== "object" || value === null) return null;
  const e = value as Record<string, unknown>;
  if (typeof e.zeit !== "string" || typeof e.sha256 !== "string") return null;
  const regeln: PasteLogRule[] = Array.isArray(e.regeln)
    ? e.regeln.flatMap((r: unknown) => {
        const item = r as Record<string, unknown> | null;
        return item && isRule(item.rule) && typeof item.count === "number"
          ? [{ rule: item.rule, count: item.count }]
          : [];
      })
    : [];
  const entry: PasteLogEntry = { zeit: e.zeit, sha256: e.sha256, regeln };
  if (typeof e.roh === "string") entry.roh = e.roh;
  if (typeof e.bereinigt === "string") entry.bereinigt = e.bereinigt;
  if (e.zuGross === true) {
    entry.zuGross = true;
    if (typeof e.zeichen === "number") entry.zeichen = e.zeichen;
  }
  return entry;
}

/** `einfuegungen` aus dem Seitenzustand lesen; fehlt das Feld, ist das Protokoll leer. */
export function parsePasteLog(value: unknown): PasteLogEntry[] {
  if (!Array.isArray(value)) return [];
  const entries: PasteLogEntry[] = [];
  for (const item of value) {
    const entry = parseEntry(item);
    if (entry) entries.push(entry);
  }
  return entries.slice(-PASTE_LOG_MAX_ENTRIES);
}

/** Anzahl der Änderungen eines Eintrags. */
export const pasteEntryChanges = (entry: PasteLogEntry): number =>
  entry.regeln.reduce((sum, r) => sum + r.count, 0);

export type PasteRestore =
  | { ok: true; text: string }
  | { ok: false; reason: "noRaw" | "empty" | "missing" | "ambiguous" };

/**
 * Rohfassung an die Stelle der bereinigten setzen. Das geht nur, wenn der
 * bereinigte Text genau einmal unverändert im aktuellen Text steht; war er
 * leer (alles entfernt), ist die Stelle nicht eindeutig.
 */
export function restoreRaw(
  current: string,
  entry: PasteLogEntry,
): PasteRestore {
  const { roh, bereinigt } = entry;
  if (roh === undefined || bereinigt === undefined)
    return { ok: false, reason: "noRaw" };
  if (bereinigt === "") return { ok: false, reason: "empty" };
  const first = current.indexOf(bereinigt);
  if (first < 0) return { ok: false, reason: "missing" };
  if (current.indexOf(bereinigt, first + 1) >= 0)
    return { ok: false, reason: "ambiguous" };
  return {
    ok: true,
    text:
      current.slice(0, first) + roh + current.slice(first + bereinigt.length),
  };
}
