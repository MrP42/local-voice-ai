import { commands } from "@/bindings";

/**
 * Dauerhafter Fortschritt (lva#152, Patrick 01.11.: „der Fortschritt muss
 * überall persistent sein, auch wenn wild hin und her gewechselt wird“).
 *
 * Zwei Ebenen:
 * - localStorage als schneller, synchroner Speicher (der Reader liest beim
 *   Rendern),
 * - `<appdata>/library/fortschritt.json` im Rust-Kern als Quelle, die einen
 *   geleerten WebView-Speicher, eine Neuinstallation und den Gerätewechsel
 *   überlebt.
 *
 * Jede Änderung geht sofort in den localStorage und gebündelt (höchstens
 * {@link FLUSH_MS}) in den Kern; beim Verbergen oder Schließen des Fensters
 * sofort. Beim Start ergänzt {@link hydrateProgress} fehlende lokale Werte aus
 * dem Kern und schiebt rein lokale (aus der Zeit vor der Bibliothek) hinauf.
 *
 * Gespiegelt werden die Weiterlese-Stellen und Lesezeichen des Readers
 * (`tts.resume.*`, `tts.bookmarks.*`) und die Fortschritts-Datensätze
 * (`progress.*`). Schlüssel der Datensätze:
 * - `text:<seite>:<reiter>` – Text einer Vorlesen-Seite (Zeichen),
 * - `audio:<buch>` – importiertes Hörbuch (Millisekunden),
 * - `doc:<buch>:<dokument>` – abgeleitete Dokumente wie Zusammenfassungen.
 */

const LS_PREFIX = "lva.ui.";
const MIRRORED = ["tts.resume.", "tts.bookmarks.", "progress."];
export const FLUSH_MS = 800;
export const HYDRATED_EVENT = "lv-progress-hydrated";
export const CHANGED_EVENT = "lv-progress-changed";

export type Range = [number, number];

export interface ProgressRecord {
  /** Aktuelle Stelle (Zeichen bzw. ms). */
  offset: number;
  /** Textanker an der Stelle (nur Text). */
  anchor?: string;
  /** Gesamtlänge (Zeichen bzw. ms); 0 = unbekannt. */
  length: number;
  /** Gehörte Bereiche, zusammengefasst und sortiert. */
  heard: Range[];
  /** Gelesene Bereiche (ohne Vorlesen umgeblättert). */
  read: Range[];
  /** Zuletzt geändert (Unix-ms). */
  updated: number;
  /** Von Hand als fertig markiert. */
  finished?: boolean;
  /** `chars` (Vorgabe) oder `ms`. */
  unit?: "chars" | "ms";
}

const storage = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

const mirrored = (key: string) => MIRRORED.some((p) => key.startsWith(p));

/**
 * Noch nicht im Kern gesicherte Änderungen. Liegen zusätzlich im localStorage
 * (außerhalb von `lva.ui.`), damit eine Löschung, die vor dem Sichern endet
 * (Fenster zu, Absturz), beim nächsten Start nicht aus dem Kern zurückkommt.
 */
const PENDING_KEY = "lva.progress.pending";
const pending = new Map<string, string | null>();
let timer: number | null = null;
let failures = 0;

function persistPending() {
  try {
    if (pending.size)
      storage()?.setItem(PENDING_KEY, JSON.stringify([...pending]));
    else storage()?.removeItem(PENDING_KEY);
  } catch {
    /* voller Speicher: bleibt im Arbeitsspeicher */
  }
}

function restorePending() {
  try {
    const raw = storage()?.getItem(PENDING_KEY);
    if (!raw) return;
    for (const [key, value] of JSON.parse(raw) as [string, string | null][])
      if (typeof key === "string" && !pending.has(key)) pending.set(key, value);
  } catch {
    /* unlesbar: verwerfen */
  }
}

async function flush(): Promise<void> {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
  if (!pending.size) return;
  const entries = [...pending].map(([key, value]) => ({ key, value }));
  pending.clear();
  try {
    const result = await commands.libraryProgressPut(entries);
    if (result.status === "error") throw new Error(result.error);
    failures = 0;
    persistPending();
  } catch {
    // Kern nicht erreichbar: zurück in die Warteschlange, später erneut.
    for (const { key, value } of entries)
      if (!pending.has(key)) pending.set(key, value);
    persistPending();
    failures = Math.min(failures + 1, 6);
    schedule(FLUSH_MS * 2 ** failures);
  }
}

export const flushProgress = flush;

function schedule(delay = FLUSH_MS) {
  if (timer !== null) return;
  timer = window.setTimeout(() => void flush(), delay);
}

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => void flush());
  window.addEventListener("beforeunload", () => void flush());
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") void flush();
  });
}

/**
 * Wert unter `lva.ui.<key>` setzen (`null` entfernt) und, wenn es ein
 * Fortschrittsschlüssel ist, in den Kern spiegeln.
 */
export function setLocal(key: string, value: string | null): void {
  try {
    if (value === null) storage()?.removeItem(LS_PREFIX + key);
    else storage()?.setItem(LS_PREFIX + key, value);
  } catch {
    /* voller Speicher: der Kern hat den Wert trotzdem */
  }
  if (!mirrored(key)) return;
  pending.set(key, value);
  persistPending();
  schedule();
}

export function getLocal(key: string): string | null {
  try {
    return storage()?.getItem(LS_PREFIX + key) ?? null;
  } catch {
    return null;
  }
}

const updatedOf = (raw: string | null): number => {
  if (!raw) return -1;
  try {
    const v = JSON.parse(raw) as { updated?: unknown };
    return typeof v?.updated === "number" ? v.updated : 0;
  } catch {
    return 0;
  }
};

/**
 * Beim Start: Kern und localStorage abgleichen. Fehlt lokal etwas, kommt es
 * aus dem Kern; ist ein lokaler Datensatz neuer (Absturz vor dem Sichern) oder
 * gibt es ihn nur lokal, geht er in den Kern.
 */
export async function hydrateProgress(): Promise<void> {
  restorePending();
  let remote: Partial<Record<string, string>> = {};
  try {
    const result = await commands.libraryProgressLoad();
    if (result.status !== "ok") return;
    remote = result.data ?? {};
  } catch {
    return;
  }
  const local = storage();
  if (!local) return;
  for (const [key, value] of Object.entries(remote)) {
    // Ungesicherte lokale Änderung (auch Löschung) hat Vorrang.
    if (value === undefined || !mirrored(key) || pending.has(key)) continue;
    const mine = local.getItem(LS_PREFIX + key);
    const merged =
      mine !== null && key.startsWith("progress.")
        ? mergeRecordText(mine, value)
        : null;
    if (merged !== null) {
      if (merged !== mine) {
        try {
          local.setItem(LS_PREFIX + key, merged);
        } catch {
          /* ignorieren */
        }
      }
      if (merged !== value) pending.set(key, merged);
    } else if (mine === null || updatedOf(value) > updatedOf(mine)) {
      try {
        local.setItem(LS_PREFIX + key, value);
      } catch {
        /* ignorieren */
      }
    } else if (mine !== value) {
      pending.set(key, mine);
    }
  }
  for (let i = 0; i < local.length; i++) {
    const full = local.key(i);
    if (!full?.startsWith(LS_PREFIX)) continue;
    const key = full.slice(LS_PREFIX.length);
    if (mirrored(key) && !(key in remote) && !pending.has(key)) {
      const value = local.getItem(full);
      if (value !== null) pending.set(key, value);
    }
  }
  if (pending.size) await flush();
  window.dispatchEvent(new Event(HYDRATED_EVENT));
}

/**
 * Zwei Fassungen desselben Datensatzes zusammenführen: gehörte und gelesene
 * Bereiche beider Seiten bleiben, Stelle und Anker vom neueren, „fertig“ wenn
 * eine Seite es sagt. `null`, wenn eine Seite kein Datensatz ist.
 */
function mergeRecordText(a: string, b: string): string | null {
  try {
    const x = JSON.parse(a) as ProgressRecord;
    const y = JSON.parse(b) as ProgressRecord;
    if (typeof x?.offset !== "number" || typeof y?.offset !== "number") return null;
    const [older, newer] = (x.updated ?? 0) >= (y.updated ?? 0) ? [y, x] : [x, y];
    const merged: ProgressRecord = {
      ...older,
      ...newer,
      heard: mergeRanges([...(older.heard ?? []), ...(newer.heard ?? [])]),
      read: mergeRanges([...(older.read ?? []), ...(newer.read ?? [])]),
      length: Math.max(newer.length ?? 0, older.length ?? 0),
    };
    if (older.finished || newer.finished) merged.finished = true;
    const text = JSON.stringify(merged);
    // Unverändert gegenüber einer Seite: deren Text behalten (kein Schreiben).
    if (text === JSON.stringify(x)) return a;
    if (text === JSON.stringify(y)) return b;
    return text;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Fortschritts-Datensätze
// ---------------------------------------------------------------------------

const recordKey = (key: string) => `progress.${key}`;

export function loadProgress(key: string): ProgressRecord | null {
  const raw = getLocal(recordKey(key));
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as ProgressRecord;
    if (typeof v?.offset !== "number" || typeof v.length !== "number")
      return null;
    return {
      ...v,
      heard: Array.isArray(v.heard) ? v.heard : [],
      read: Array.isArray(v.read) ? v.read : [],
    };
  } catch {
    return null;
  }
}

export function saveProgress(key: string, record: ProgressRecord): void {
  setLocal(recordKey(key), JSON.stringify(record));
  window.dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: { key } }));
}

export function removeProgress(key: string): void {
  setLocal(recordKey(key), null);
  window.dispatchEvent(new CustomEvent(CHANGED_EVENT, { detail: { key } }));
}

/** Bereiche zusammenfassen: sortiert, überlappende und anstoßende vereint. */
export function mergeRanges(ranges: Range[]): Range[] {
  const sorted = ranges
    .filter(([a, b]) => Number.isFinite(a) && Number.isFinite(b) && b > a)
    .map(([a, b]) => [a, b] as Range)
    .sort((x, y) => x[0] - y[0]);
  const out: Range[] = [];
  for (const r of sorted) {
    const last = out[out.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else out.push(r);
  }
  return out;
}

const covered = (ranges: Range[]) =>
  mergeRanges(ranges).reduce((sum, [a, b]) => sum + (b - a), 0);

/** Anteil 0…1: gehört oder gelesen, was davon mehr ist; „fertig“ = 1. */
export function progressFraction(rec: ProgressRecord | null): number {
  if (!rec) return 0;
  if (rec.finished) return 1;
  if (rec.length <= 0) return 0;
  const done = covered([...rec.heard, ...rec.read]);
  return Math.max(0, Math.min(1, Math.max(done, rec.offset) / rec.length));
}

/** Größter Sprung, der noch als „durchgehört“ zählt (Zeichen bzw. ms). */
const MAX_STEP = { chars: 4000, ms: 120_000 };

/**
 * Wiedergabe ist von `from` nach `to` gelaufen. Kleine Schritte werden als
 * gehörter Bereich gezählt, große Sprünge (Kapitelsprung, Suchen) nur als
 * neue Stelle.
 */
export function recordPosition(
  key: string,
  opts: {
    to: number;
    length: number;
    anchor?: string;
    unit?: "chars" | "ms";
    mode?: "heard" | "read";
  },
): ProgressRecord {
  const unit = opts.unit ?? "chars";
  const prev = loadProgress(key);
  const from = prev?.offset ?? 0;
  const rec: ProgressRecord = {
    offset: Math.max(0, opts.to),
    anchor: opts.anchor,
    length: opts.length > 0 ? opts.length : (prev?.length ?? 0),
    heard: prev?.heard ?? [],
    read: prev?.read ?? [],
    updated: Date.now(),
    finished: prev?.finished,
    unit,
  };
  const step = opts.to - from;
  if (step > 0 && step <= MAX_STEP[unit]) {
    const field = opts.mode === "read" ? "read" : "heard";
    rec[field] = mergeRanges([...rec[field], [from, opts.to]]);
  }
  // Fast alles gehört oder gelesen: zählt als fertig.
  if (rec.length > 0 && covered([...rec.heard, ...rec.read]) >= rec.length * 0.98)
    rec.finished = true;
  saveProgress(key, rec);
  return rec;
}

/** Ein Bereich wurde gelesen (Umblättern ohne Vorlesen). */
export function recordRead(
  key: string,
  from: number,
  to: number,
  length: number,
): ProgressRecord {
  const prev = loadProgress(key);
  const rec: ProgressRecord = {
    offset: Math.max(prev?.offset ?? 0, 0),
    anchor: prev?.anchor,
    length: length > 0 ? length : (prev?.length ?? 0),
    heard: prev?.heard ?? [],
    read: mergeRanges([...(prev?.read ?? []), [from, to]]),
    updated: Date.now(),
    finished: prev?.finished,
    unit: prev?.unit ?? "chars",
  };
  rec.offset = Math.max(rec.offset, to);
  saveProgress(key, rec);
  return rec;
}

export function setFinished(key: string, finished: boolean): void {
  const prev = loadProgress(key);
  saveProgress(key, {
    offset: prev?.offset ?? 0,
    anchor: prev?.anchor,
    length: prev?.length ?? 0,
    heard: prev?.heard ?? [],
    read: prev?.read ?? [],
    updated: Date.now(),
    finished,
    unit: prev?.unit,
  });
}

/** Schlüssel für den Originaltext einer Vorlesen-Seite. */
export const textProgressKey = (pageId: string, tab = "original") =>
  `text:${pageId}:${tab}`;
export const audioProgressKey = (bookId: string) => `audio:${bookId}`;
export const docProgressKey = (bookId: string, docId: string) =>
  `doc:${bookId}:${docId}`;
