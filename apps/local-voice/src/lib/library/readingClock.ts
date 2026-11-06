/**
 * Taktgeber der Lesestatistik (lva#354). Genau einer je Fenster: eine neue
 * Quelle (Hörbuch, Vorlesen, Lesen) beendet die vorige, damit Mitlesen und
 * Hören nie doppelt zählen (Codex A1).
 *
 * - Wiedergabe zählt Wandzeit, solange sie läuft (auch bei verborgenem Fenster).
 * - Lesen zählt nur bei sichtbarem Fenster und Aktivität in den letzten 3 Min.
 * - Lücken über 15 s beim Lesen bzw. 2 min bei Wiedergabe (gedrosselte Timer
 *   im Hintergrund) zählen nicht (Schlaf, Hänger).
 * - Zuwächse gehen gebündelt (alle 10 s, beim Verbergen sofort) an den Kern.
 */
import { commands, type ReadingDelta } from "@/bindings";
import { localDay, type Format } from "./readingStats";

export const TICK_MS = 5_000;
export const MAX_GAP_MS = 15_000;
/** Wiedergabe: verborgene Fenster drosseln Timer (bis ~1/min), daher großzügiger. */
export const MAX_GAP_PLAYBACK_MS = 120_000;
export const IDLE_MS = 180_000;
export const FLUSH_MS = 10_000;
export const CHANGED_EVENT = "lv-reading-log-changed";

export interface ReadingSession {
  bookId: string;
  format: Format;
  /** Sprachcode der gelesenen/gehörten Fassung. */
  lang: string;
}

const LANG_ALIASES: Record<string, string> = {
  ger: "de",
  deu: "de",
  german: "de",
  deutsch: "de",
  eng: "en",
  english: "en",
  englisch: "en",
  fre: "fr",
  fra: "fr",
  spa: "es",
  ita: "it",
  dut: "nl",
  nld: "nl",
  por: "pt",
  pol: "pl",
  tur: "tr",
};

/** Sprachangabe eines Buchs auf einen kurzen Code bringen, sonst `und`. */
export function normalizeLang(raw: string | null | undefined): string {
  const s = (raw ?? "").trim().toLowerCase();
  if (!s) return "und";
  if (LANG_ALIASES[s]) return LANG_ALIASES[s];
  const primary = s.split(/[-_]/)[0];
  if (LANG_ALIASES[primary]) return LANG_ALIASES[primary];
  return /^[a-z]{2,3}$/.test(primary) ? primary : "und";
}

type Pending = Map<string, ReadingDelta>;

interface Clock {
  current: ReadingSession | null;
  lastTick: number;
  lastActivity: number;
  pending: Pending;
  lastFlush: number;
  timer: ReturnType<typeof setInterval> | null;
  flushing: boolean;
}

const clock: Clock = {
  current: null,
  lastTick: 0,
  lastActivity: 0,
  pending: new Map(),
  lastFlush: 0,
  timer: null,
  flushing: false,
};

let now = () => Date.now();
let visible = () =>
  typeof document === "undefined" || document.visibilityState === "visible";

/** Nur für Tests: Uhr und Sichtbarkeit ersetzen, Zustand leeren. */
export function __setClockForTests(
  clockFn: () => number,
  visibleFn: () => boolean,
): void {
  now = clockFn;
  visible = visibleFn;
  clock.current = null;
  clock.pending.clear();
  clock.lastTick = clockFn();
  clock.lastActivity = clockFn();
  clock.lastFlush = clockFn();
}

/** Nur für Tests: wartende Zuwächse ansehen. */
export const __pendingForTests = (): ReadingDelta[] => [
  ...clock.pending.values(),
];

const sameSession = (a: ReadingSession | null, b: ReadingSession | null) =>
  !!a &&
  !!b &&
  a.bookId === b.bookId &&
  a.format === b.format &&
  a.lang === b.lang;

function bucket(s: ReadingSession, at: number): ReadingDelta {
  const day = localDay(new Date(at));
  const key = `${day}|${s.bookId}|${s.format}|${s.lang}`;
  let d = clock.pending.get(key);
  if (!d) {
    d = {
      day,
      book_id: s.bookId,
      format: s.format,
      lang: s.lang,
      sec: 0,
      chars: 0,
      audio_ms: 0,
    };
    clock.pending.set(key, d);
  }
  return d;
}

/** Zeit seit dem letzten Takt der laufenden Sitzung gutschreiben. */
export function accrue(): void {
  const t = now();
  const s = clock.current;
  const gap = t - clock.lastTick;
  clock.lastTick = t;
  if (!s || gap <= 0) return;
  if (gap > (s.format === "lesen" ? MAX_GAP_MS : MAX_GAP_PLAYBACK_MS)) return;
  if (s.format === "lesen" && (!visible() || t - clock.lastActivity > IDLE_MS))
    return;
  bucket(s, t).sec += gap / 1000;
}

function ensureTimer(): void {
  if (clock.timer || typeof window === "undefined") return;
  clock.timer = setInterval(() => {
    accrue();
    if (now() - clock.lastFlush >= FLUSH_MS) void flushReadingLog();
  }, TICK_MS);
  const hide = () => {
    accrue();
    void flushReadingLog();
  };
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") hide();
    else clock.lastTick = now();
  });
  window.addEventListener("pagehide", hide);
  for (const ev of ["keydown", "pointerdown", "wheel", "scroll", "touchstart"])
    window.addEventListener(ev, noteReadingActivity, {
      passive: true,
      capture: true,
    });
}

/** Eine Quelle beginnt (oder läuft weiter). Beendet eine andere Quelle. */
export function startReading(s: ReadingSession): void {
  if (!s.bookId) return;
  ensureTimer();
  if (sameSession(clock.current, s)) return;
  accrue();
  clock.current = { ...s };
  clock.lastTick = now();
  clock.lastActivity = now();
}

/**
 * Eine Quelle endet. Mit `match` nur, wenn sie die laufende ist (eine
 * pausierte Hörbuchleiste beendet kein Vorlesen, das inzwischen läuft).
 */
export function stopReading(match?: Partial<ReadingSession>): void {
  const c = clock.current;
  if (!c) return;
  if (match?.bookId && match.bookId !== c.bookId) return;
  if (match?.format && match.format !== c.format) return;
  accrue();
  clock.current = null;
}

export const currentReading = (): ReadingSession | null =>
  clock.current ? { ...clock.current } : null;

/** Blättern, Scrollen, Tippen: Lesen gilt wieder als aktiv. */
export function noteReadingActivity(): void {
  clock.lastActivity = now();
}

/** Fortschritt (neu gelesene Zeichen, gehörte ms) einer Quelle gutschreiben. */
export function addReadingProgress(
  s: ReadingSession,
  p: { chars?: number; audioMs?: number },
): void {
  if (!s.bookId) return;
  const chars = Math.max(0, Math.round(p.chars ?? 0));
  const audio = Math.max(0, Math.round(p.audioMs ?? 0));
  if (!chars && !audio) return;
  const d = bucket(s, now());
  d.chars = (d.chars ?? 0) + chars;
  d.audio_ms = (d.audio_ms ?? 0) + audio;
}

let inflight: Promise<void> | null = null;

/**
 * Wartende Zuwächse an den Kern geben. Schlägt es fehl, bleiben sie. Läuft
 * schon eine Meldung, wird erst auf sie gewartet (Löschen verlässt sich
 * darauf, dass danach nichts mehr aussteht).
 */
export async function flushReadingLog(): Promise<void> {
  clock.lastFlush = now();
  while (inflight) await inflight;
  if (clock.pending.size === 0) return;
  inflight = sendPending().finally(() => {
    inflight = null;
  });
  return inflight;
}

async function sendPending(): Promise<void> {
  const batch = [...clock.pending.values()]
    .map((d) => ({ ...d, sec: Math.round(d.sec) }))
    .filter((d) => d.sec > 0 || (d.chars ?? 0) > 0 || (d.audio_ms ?? 0) > 0);
  // Sekundenreste bleiben für die nächste Meldung stehen.
  const rest = new Map<string, ReadingDelta>();
  for (const [k, d] of clock.pending) {
    const r = d.sec - Math.round(d.sec);
    if (r > 0) rest.set(k, { ...d, sec: r, chars: 0, audio_ms: 0 });
  }
  if (batch.length === 0) return;
  clock.flushing = true;
  clock.pending = rest;
  try {
    const res = await commands.readingLogAdd(batch);
    if (res.status === "error") throw new Error(res.error);
    if (typeof window !== "undefined")
      window.dispatchEvent(new CustomEvent(CHANGED_EVENT));
  } catch {
    // Zurücklegen: beim nächsten Mal erneut.
    for (const d of batch) {
      const key = `${d.day}|${d.book_id}|${d.format}|${d.lang}`;
      const cur = clock.pending.get(key);
      if (cur) {
        cur.sec += d.sec;
        cur.chars = (cur.chars ?? 0) + (d.chars ?? 0);
        cur.audio_ms = (cur.audio_ms ?? 0) + (d.audio_ms ?? 0);
      } else clock.pending.set(key, { ...d });
    }
  } finally {
    clock.flushing = false;
  }
}
