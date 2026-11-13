/**
 * Rohfassung eingefügter Texte (lva#530 I1, Konzept §4.1).
 *
 * Vor dem Bereinigen wird der eingefügte Rohtext in der Zustandsdatei der
 * Seite (`state.json`, Feld `einfuegungen`) gesichert. So bleibt er nach einem
 * Neustart verfügbar und lässt sich zurückholen, auch wenn Strg+Z längst
 * nichts mehr weiß. Alte Zustände ohne das Feld bleiben gültig. Reine Logik,
 * kein Zugriff auf Dateien; Inhalte gehen nie ins Log.
 *
 * Grenzen gelten in Bytes (UTF-8 des als JSON gespeicherten Eintrags), nicht
 * in Zeichen: „ä“ oder Emoji belegen mehr als ein Byte.
 */
import type { PasteRule } from "./pasteCleanup.ts";

/** Höchstens so viele Einfügungen je Seite (die ältesten fallen zuerst). */
export const PASTE_LOG_MAX_ENTRIES = 20;
/** Größter Eintrag (UTF-8-Bytes des JSON): Roh- plus bereinigte Fassung. */
export const PASTE_ENTRY_MAX_BYTES = 2 * 1024 * 1024;
/** Obergrenze für alle Einträge zusammen (state.json wird bei jeder Änderung
 *  ganz geschrieben und synchronisiert). */
export const PASTE_LOG_MAX_BYTES = 16 * 1024 * 1024;

export interface PasteLogRule {
  rule: PasteRule;
  count: number;
}

export interface PasteLogEntry {
  /** Zeitpunkt des Einfügens, ISO 8601. */
  zeit: string;
  /** SHA-256 (hex) des Rohtexts mit vereinheitlichten Zeilenenden. */
  sha256: string;
  /** Rohtext; fehlt, wenn der Eintrag zu groß war. */
  roh?: string;
  /** Bereinigter Text, wie er eingefügt wurde; fehlt, wenn zu groß. */
  bereinigt?: string;
  /** Änderungen je Regel. */
  regeln: PasteLogRule[];
  /** Zu groß für Sicherung und Bereinigung: nur Prüfsumme und Größe bleiben. */
  zuGross?: true;
  /** Länge des Rohtexts in Zeichen (nur bei `zuGross`). */
  zeichen?: number;
  /** Größe des Rohtexts in UTF-8-Bytes (nur bei `zuGross`). */
  bytes?: number;
  /** Nur zur Laufzeit (nie gespeichert): Ergebnis der Prüfsummenprüfung. Fehlt
   *  es, wurde noch nicht geprüft; das gilt wie „nicht prüfbar“. */
  pruefung?: PasteCheck;
}

/** ok: Prüfsumme stimmt; damaged: sie passt nicht (oder fehlt trotz WebCrypto);
 *  unverifiable: die Umgebung kann sie nicht berechnen (kein WebCrypto, Fehler). */
export type PasteCheck = "ok" | "damaged" | "unverifiable";

const RULES: readonly PasteRule[] = [
  "repeated",
  "control",
  "pageNumbers",
  "webNoise",
  "fragments",
  "spaces",
  "blankLines",
];

const encoder = new TextEncoder();
const normalizeNewlines = (text: string) => text.replace(/\r\n?/g, "\n");
/** UTF-8-Länge eines Textes in Bytes. */
export const utf8Bytes = (text: string): number => encoder.encode(text).length;

/** Größe eines Eintrags: UTF-8-Länge seines JSON. */
export const entryBytes = (entry: PasteLogEntry): number =>
  utf8Bytes(JSON.stringify(entry));

/** SHA-256 als Hex-Text; leer, wenn die Umgebung kein WebCrypto bietet. */
export async function sha256Hex(text: string): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) return "";
  const digest = await subtle.digest("SHA-256", encoder.encode(text));
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

/** Eintrag für eine bereinigte Einfügung (kann größer als erlaubt sein, siehe {@link fitsEntry}). */
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

/** Eintrag für einen zu großen Text: nur Prüfsumme und Größe, keine Rohfassung. */
export async function makeOversizedEntry(
  raw: string,
  now: Date = new Date(),
): Promise<PasteLogEntry> {
  const roh = normalizeNewlines(raw);
  return {
    zeit: now.toISOString(),
    sha256: await sha256Hex(roh),
    regeln: [],
    zuGross: true,
    zeichen: roh.length,
    bytes: utf8Bytes(roh),
  };
}

/** Passt der Eintrag in die Größengrenze für einen einzelnen Eintrag? */
export const fitsEntry = (entry: PasteLogEntry): boolean =>
  entryBytes(entry) <= PASTE_ENTRY_MAX_BYTES;

/** Übergroßer Eintrag verliert Roh- und bereinigte Fassung; sha und Größe bleiben. */
function shrink(entry: PasteLogEntry): PasteLogEntry {
  const small: PasteLogEntry = {
    zeit: entry.zeit,
    sha256: entry.sha256,
    regeln: [],
    zuGross: true,
  };
  const zeichen = entry.zeichen ?? entry.roh?.length;
  const bytes =
    entry.bytes ?? (entry.roh === undefined ? undefined : utf8Bytes(entry.roh));
  if (zeichen !== undefined) small.zeichen = zeichen;
  if (bytes !== undefined) small.bytes = bytes;
  return small;
}

/**
 * Eintrag auf die Einzelgrenze bringen: zu groß verliert Roh- und bereinigte
 * Fassung. Passt auch das Gekürzte nicht (Metadaten allein zu groß), fällt der
 * Eintrag ganz weg (`null`), statt die Grenze zu reißen.
 */
function limitEntry(entry: PasteLogEntry): PasteLogEntry | null {
  if (fitsEntry(entry)) return entry;
  const small = shrink(entry);
  return fitsEntry(small) ? small : null;
}

/** Anzahl und Gesamtumfang begrenzen (Metadaten zählen mit): die ältesten
 *  Einträge fallen zuerst. Jeder Eintrag ist schon höchstens
 *  {@link PASTE_ENTRY_MAX_BYTES} groß, so bleibt immer der neueste stehen. */
function limitLog(entries: PasteLogEntry[]): PasteLogEntry[] {
  const next = entries.slice(-PASTE_LOG_MAX_ENTRIES);
  const sizes = next.map(entryBytes);
  let total = sizes.reduce((sum, n) => sum + n, 0);
  let from = 0;
  while (from < next.length && total > PASTE_LOG_MAX_BYTES) {
    total -= sizes[from];
    from += 1;
  }
  return next.slice(from);
}

/** Neuen Eintrag anhängen (neueste zuletzt); Grenzen für Anzahl und Umfang einhalten. */
export function appendPasteEntry(
  log: readonly PasteLogEntry[],
  entry: PasteLogEntry,
): PasteLogEntry[] {
  const fitted = limitEntry(entry);
  return limitLog(fitted ? [...log, fitted] : [...log]);
}

/** Schlüssel zum Löschen eines Eintrags. */
export const pasteEntryKey = (entry: PasteLogEntry): string =>
  `${entry.zeit}|${entry.sha256}`;

/** Einen Eintrag aus dem Protokoll nehmen (der erste mit diesem Schlüssel). */
export function removePasteEntry(
  log: readonly PasteLogEntry[],
  key: string,
): PasteLogEntry[] {
  const at = log.findIndex((e) => pasteEntryKey(e) === key);
  return at < 0 ? [...log] : [...log.slice(0, at), ...log.slice(at + 1)];
}

const ISO_TIME =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?(?:Z|[+-](\d{2}):(\d{2}))$/;
/**
 * Zeitstempel eines Eintrags: ISO 8601, höchstens 40 Zeichen, mit echtem
 * Kalenderdatum (30.02. und 29.02. im Nichtschaltjahr sind ungültig); Offset
 * und Millisekunden sind erlaubt.
 */
export function isIsoTime(value: string): boolean {
  if (value.length > 40) return false;
  const m = ISO_TIME.exec(value);
  if (!m) return false;
  const [year, month, day, hour, minute, second] = m.slice(1, 7).map(Number);
  if (hour > 23 || minute > 59 || second > 59) return false;
  if (m[7] !== undefined && (Number(m[7]) > 23 || Number(m[8]) > 59))
    return false;
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  return (
    date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day &&
    !Number.isNaN(Date.parse(value))
  );
}
/** SHA-256 als 64 Hex-Zeichen; leer nur bei einem Eintrag ohne Rohfassung
 *  (mit Rohfassung gilt eine leere Prüfsumme als beschädigt, siehe {@link verifyPasteLog}). */
export const isSha256 = (value: string): boolean =>
  value === "" || /^[0-9a-f]{64}$/i.test(value);

const isRule = (value: unknown): value is PasteRule =>
  typeof value === "string" && (RULES as readonly string[]).includes(value);

const isCount = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

/** Eintrag aus gespeichertem JSON prüfen; Unbrauchbares verwerfen. */
function parseEntry(value: unknown): PasteLogEntry | null {
  if (typeof value !== "object" || value === null) return null;
  const e = value as Record<string, unknown>;
  if (typeof e.zeit !== "string" || !isIsoTime(e.zeit)) return null;
  if (typeof e.sha256 !== "string" || !isSha256(e.sha256)) return null;
  // Höchstens eine Angabe je Regel, nur die ersten RULES.length Elemente lesen.
  const seen = new Set<PasteRule>();
  const regeln: PasteLogRule[] = Array.isArray(e.regeln)
    ? e.regeln.slice(0, RULES.length * 2).flatMap((r: unknown) => {
        const item = r as Record<string, unknown> | null;
        if (
          !item ||
          !isRule(item.rule) ||
          seen.has(item.rule) ||
          typeof item.count !== "number" ||
          !Number.isSafeInteger(item.count) ||
          item.count < 0
        )
          return [];
        seen.add(item.rule);
        return [{ rule: item.rule, count: item.count }];
      })
    : [];
  // Hex-Schreibweise vereinheitlichen: derselbe Hash gilt in Groß- wie Kleinbuchstaben.
  const entry: PasteLogEntry = {
    zeit: e.zeit,
    sha256: e.sha256.toLowerCase(),
    regeln,
  };
  if (typeof e.roh === "string") entry.roh = e.roh;
  if (typeof e.bereinigt === "string") entry.bereinigt = e.bereinigt;
  if (e.zuGross === true) {
    entry.zuGross = true;
    if (isCount(e.zeichen)) entry.zeichen = e.zeichen;
    if (isCount(e.bytes)) entry.bytes = e.bytes;
  }
  return entry;
}

/**
 * `einfuegungen` aus dem Seitenzustand lesen; fehlt das Feld, ist das
 * Protokoll leer. Die Grenzen gelten auch hier (ein anderes Gerät oder eine
 * ältere Fassung kann mehr gespeichert haben): übergroße Einträge verlieren
 * Roh- und bereinigte Fassung, danach fallen die ältesten Einträge, bis der
 * Gesamtumfang passt.
 */
export function parsePasteLog(value: unknown): PasteLogEntry[] {
  if (!Array.isArray(value)) return [];
  const entries: PasteLogEntry[] = [];
  for (const item of value) {
    const entry = parseEntry(item);
    const fitted = entry ? limitEntry(entry) : null;
    if (fitted) entries.push(fitted);
  }
  return limitLog(entries);
}

/**
 * Prüfsummen gegen die gespeicherte Rohfassung halten und das Ergebnis je
 * Eintrag mit Rohfassung vermerken (nur zur Laufzeit). „Nicht prüfbar“ ist
 * nicht „unbeschädigt“: ohne WebCrypto oder wenn `digest` scheitert, gilt der
 * Eintrag als `unverifiable`; eine leere Prüfsumme gilt bei vorhandenem
 * WebCrypto als `damaged`. Die Funktion wirft nie.
 */
export async function verifyPasteLog(
  entries: readonly PasteLogEntry[],
): Promise<PasteLogEntry[]> {
  const subtle = globalThis.crypto?.subtle;
  const mark = (entry: PasteLogEntry, pruefung: PasteCheck): PasteLogEntry => ({
    ...entry,
    pruefung,
  });
  return Promise.all(
    entries.map(async (entry) => {
      if (entry.roh === undefined) return entry;
      if (!subtle) return mark(entry, "unverifiable");
      if (entry.sha256 === "") return mark(entry, "damaged");
      try {
        const actual = await sha256Hex(entry.roh);
        if (actual === "") return mark(entry, "unverifiable");
        return mark(entry, actual === entry.sha256 ? "ok" : "damaged");
      } catch {
        return mark(entry, "unverifiable");
      }
    }),
  );
}

/** Alle Einträge mit Rohfassung als nicht prüfbar vermerken (Prüfung gescheitert). */
export const markUnverifiable = (
  entries: readonly PasteLogEntry[],
): PasteLogEntry[] =>
  entries.map((e) =>
    e.roh === undefined ? e : { ...e, pruefung: "unverifiable" as const },
  );

/**
 * Früher Schutz vor dem Bereinigen: ein Rohtext, dessen UTF-8-Bytes allein
 * schon die Einzelgrenze sprengen, passt nie in einen Eintrag (der außerdem
 * die bereinigte Fassung und JSON-Maskierung enthält). Grenzen zählen Bytes,
 * nicht UTF-16-Einheiten: Einheiten sind nie mehr als Bytes (darüber sicher
 * zu groß), bis zu 3 Byte je Einheit genügt die Obergrenze, erst dazwischen
 * wird exakt gezählt.
 */
export function rawExceedsEntryLimit(raw: string): boolean {
  if (raw.length > PASTE_ENTRY_MAX_BYTES) return true;
  if (raw.length * 3 <= PASTE_ENTRY_MAX_BYTES) return false;
  return utf8Bytes(raw) > PASTE_ENTRY_MAX_BYTES;
}

/** Anzahl der Änderungen eines Eintrags. */
export const pasteEntryChanges = (entry: PasteLogEntry): number =>
  entry.regeln.reduce((sum, r) => sum + r.count, 0);

export type PasteRestore =
  | { ok: true; text: string }
  | {
      ok: false;
      reason:
        | "noRaw"
        | "damaged"
        | "unverifiable"
        | "empty"
        | "missing"
        | "ambiguous";
    };

/**
 * Rohfassung an die Stelle der bereinigten setzen. Das geht nur, wenn die
 * Prüfsumme geprüft stimmt (`pruefung === "ok"`; beschädigt und nicht prüfbar
 * sind gesperrt) und der bereinigte Text genau einmal unverändert
 * im aktuellen Text steht; war er leer (alles entfernt), ist die Stelle nicht
 * eindeutig.
 */
export function restoreRaw(
  current: string,
  entry: PasteLogEntry,
): PasteRestore {
  const { roh, bereinigt } = entry;
  if (roh === undefined || bereinigt === undefined)
    return { ok: false, reason: "noRaw" };
  if (entry.pruefung === "damaged") return { ok: false, reason: "damaged" };
  if (entry.pruefung !== "ok") return { ok: false, reason: "unverifiable" };
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
