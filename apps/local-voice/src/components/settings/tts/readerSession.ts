import { listen } from "@tauri-apps/api/event";
import { sentenceAfterProgress } from "./sentenceCounter";
import { beginReadingSession, flushProgress, type ReadingLogEntry } from "@/lib/library/progress";
import { SentenceLocator } from "@/lib/reader/sentenceLocator";
import type { ReaderBlock } from "@/lib/reader/readerDoc";
import {
  startPiece,
  cleanPieces,
  cleanSpans,
  type SpeechSpan,
} from "@/lib/speechTiming";

/** Beginn, Dauer und Sprechzeit des hörbaren Satzes (Wort- und Zeitlauf). */
export interface SentenceStart {
  index: number | null;
  /** `performance.now()` beim Start des Satzes. */
  at: number;
  /** Satzzeit, die bei `at` schon gespielt war (Stückbeginn, Tempowechsel). */
  base?: number;
  ms: number;
  /** Stimmhafte Abschnitte (ms bei Tempo 1); leer bei älterem Kern. */
  speech?: SpeechSpan[];
  /** Beginn jedes Stücks (ms bei Tempo 1); die Uhr hält an der Grenze. */
  pieces?: number[];
  /** Beginn des zuletzt gemeldeten Stücks; nur `tts-sentence-piece` rückt es vor. */
  piece?: number;
}

/** Nächstes Stück desselben Satzes: die Uhr setzt am echten Start nach. */
export function sentencePieceStarted(
  start: SentenceStart | null,
  payload: { index?: number; offset_ms?: number },
  now: number,
): SentenceStart | null {
  if (
    !start ||
    typeof payload.index !== "number" ||
    start.index !== payload.index
  )
    return start;
  const offset = payload.offset_ms;
  if (typeof offset !== "number" || !Number.isFinite(offset)) return start;
  return startPiece(start, now, Math.min(offset, start.ms));
}

/**
 * Die laufende Vorlese-Sitzung der Vorlesen-Seite, unabhängig von der Komponente.
 *
 * Die Seite wird beim Wechsel in einen anderen App-Bereich ausgehängt, die
 * Wiedergabe im Kern läuft aber weiter. Lägen Reiter, Fortschritt und Satz nur in
 * der Komponente, wüsste die Seite nach der Rückkehr nicht mehr, was gelesen wird:
 * keine Hervorhebung, und Play nach einer Pause begänne an der alten gemerkten
 * Stelle statt fortzusetzen (Patrick 02.11.2026). Deshalb steht hier, was eine
 * Sitzung über das Aushängen hinaus ausmacht; die Ereignisse des Kerns werden
 * auch ohne offene Seite mitgeschrieben.
 */
export type SessionTab = "original" | "translation" | "summary" | "term";

export interface ReaderSession {
  /** Gehört die Wiedergabe im Kern gerade der Vorlesen-Seite? Liest jemand
   *  anderes (Aufnahmen), schreibt diese Sitzung nichts mit. */
  active: boolean;
  /** Reiter, zu dem die Sitzung gehört. */
  tab: SessionTab;
  /** KI-Dokument der Sitzung (nur beim Reiter „summary“). */
  docId: string | null;
  /** Schlüssel der gemerkten Stelle, die die Sitzung fortschreibt. */
  resumeKey: string | null;
  /** Seite der zuletzt gestarteten Sitzung — Play nach Pause setzt sie fort. */
  lastPage: string | null;
  /** Gesprochene Sätze und Gesamtzahl. */
  progress: { position: number; total: number } | null;
  /** Der Satz, der zu hören ist. */
  sentence: string | null;
  /** Nullbasierter Index dieses Satzes. */
  index: number | null;
  /** Beginn und Dauer des hörbaren Satzes (Wort- und Zeitlauf). */
  sentenceStart: SentenceStart | null;
  /** Anfang der zuletzt hervorgehobenen Stelle im Text der Sitzung: dort sucht
   *  die Seite nach der Rückkehr den aktuellen Satz, nicht am Textanfang. */
  rangeStart: number | null;
}

export const readerSession: ReaderSession = {
  active: false,
  tab: "original",
  docId: null,
  resumeKey: null,
  lastPage: null,
  progress: null,
  sentence: null,
  index: null,
  sentenceStart: null,
  rangeStart: null,
};

/** Ein Feld der Sitzung als Ref: die Seite liest und schreibt es wie `useRef`. */
export const sessionRef = <K extends keyof ReaderSession>(key: K) => ({
  get current(): ReaderSession[K] {
    return readerSession[key];
  },
  set current(value: ReaderSession[K]) {
    readerSession[key] = value;
  },
});

/**
 * Die Wiedergabe gehört nicht mehr der Vorlesen-Seite (Stopp, eine
 * Begriffserklärung oder eine andere Stelle der App liest): alles vergessen,
 * damit die Seite beim nächsten Öffnen nichts Fremdes markiert oder fortsetzt.
 */
export const endReaderSession = () => {
  endReaderReading();
  readingSource = null;
  readerSession.active = false;
  readerSession.progress = null;
  readerSession.sentence = null;
  readerSession.index = null;
  readerSession.sentenceStart = null;
  readerSession.rangeStart = null;
};

let listening = false;
let readingSource: {
  bookId: string;
  edition: string;
  medium: ReadingLogEntry["medium"];
  locator: SentenceLocator;
  position: number;
  sentence: string | null;
  index: number | null;
} | null = null;
let readingCheckpoint: ((to: number) => void) | null = null;
let readingTimer: ReturnType<typeof window.setInterval> | null = null;

const checkpointReaderReading = () => {
  if (readingSource) readingCheckpoint?.(readingSource.position);
};

const endReaderReading = () => {
  checkpointReaderReading();
  readingCheckpoint = null;
  if (readingTimer !== null) window.clearInterval(readingTimer);
  readingTimer = null;
  void flushProgress();
};

/** Quelle beim Start festhalten; Reiter und Komponentenlebensdauer spielen danach keine Rolle. */
export function startReaderReading(
  bookId: string | null, edition: string, medium: ReadingLogEntry["medium"],
  blocks: ReaderBlock[], from: number,
): void {
  endReaderReading();
  readingSource = bookId ? { bookId, edition, medium, locator: new SentenceLocator(blocks), position: from, sentence: null, index: null } : null;
  readingSource?.locator.reset(from);
}

const beginReaderReading = () => {
  if (!readingSource || readingCheckpoint) return;
  const { bookId, edition, medium, position } = readingSource;
  readingCheckpoint = beginReadingSession(bookId, { edition, medium, from: position, unit: "chars" });
  readingTimer = window.setInterval(checkpointReaderReading, 5000);
};

/**
 * Fortschritt und Satz einmal und dauerhaft mitschreiben — auch, solange die
 * Vorlesen-Seite nicht offen ist. Dieselbe Fortschreibung wie in der Seite.
 */
export const ensureReaderSessionListeners = () => {
  if (listening) return;
  listening = true;
  window.addEventListener("pagehide", endReaderReading);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") { checkpointReaderReading(); void flushProgress(); }
  });
  void listen<{ phase: string }>("tts-state-changed", (e) => {
    if (readerSession.active && e.payload.phase === "speaking") beginReaderReading();
    else endReaderReading();
  });
  void listen<{ position: number; total: number }>(
    "tts-speak-progress",
    (e) => {
      if (!readerSession.active) return;
      readerSession.progress = e.payload;
      const index = readerSession.index;
      readerSession.index =
        index === null
          ? null
          : (sentenceAfterProgress({ index }, e.payload.position)?.index ??
            null);
    },
  );
  void listen<{
    context: string;
    index?: number;
    text: string;
    duration_ms?: number;
    speech_ms?: unknown;
    pieces_ms?: unknown;
  }>("tts-current-sentence", (e) => {
    if (e.payload.context !== "speak" || !readerSession.active) return;
    const index = typeof e.payload.index === "number" ? e.payload.index : null;
    if (readingSource && (readingSource.sentence !== e.payload.text || readingSource.index !== index)) {
      if (index !== null && readingSource.index !== null && index < readingSource.index) readingSource.locator.reset(0);
      const range = readingSource.locator.locate(e.payload.text);
      if (range) readingSource.position = range.start;
      readingSource.sentence = e.payload.text;
      readingSource.index = index;
      checkpointReaderReading();
    }
    readerSession.sentenceStart = {
      index,
      at: performance.now(),
      ms: e.payload.duration_ms ?? 0,
      speech: cleanSpans(e.payload.speech_ms, e.payload.duration_ms),
      pieces: cleanPieces(e.payload.pieces_ms, e.payload.duration_ms),
    };
    readerSession.sentence = e.payload.text;
    readerSession.index = index;
  });
  void listen<{ context: string; index?: number; offset_ms?: number }>(
    "tts-sentence-piece",
    (e) => {
      if (e.payload.context !== "speak" || !readerSession.active) return;
      readerSession.sentenceStart = sentencePieceStarted(
        readerSession.sentenceStart,
        e.payload,
        performance.now(),
      );
    },
  );
};
