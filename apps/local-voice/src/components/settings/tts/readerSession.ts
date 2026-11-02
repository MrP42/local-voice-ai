import { listen } from "@tauri-apps/api/event";
import { sentenceAfterProgress } from "./sentenceCounter";

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
  sentenceStart: { index: number | null; at: number; ms: number } | null;
  /** Anfang der zuletzt hervorgehobenen Stelle im Text der Sitzung: dort sucht
   *  die Seite nach der Rückkehr den aktuellen Satz, nicht am Textanfang. */
  rangeStart: number | null;
}

export const readerSession: ReaderSession = {
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

let listening = false;
/**
 * Fortschritt und Satz einmal und dauerhaft mitschreiben — auch, solange die
 * Vorlesen-Seite nicht offen ist. Dieselbe Fortschreibung wie in der Seite.
 */
export const ensureReaderSessionListeners = () => {
  if (listening) return;
  listening = true;
  void listen<{ position: number; total: number }>(
    "tts-speak-progress",
    (e) => {
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
  }>("tts-current-sentence", (e) => {
    if (e.payload.context !== "speak") return;
    const index = typeof e.payload.index === "number" ? e.payload.index : null;
    readerSession.sentenceStart = {
      index,
      at: performance.now(),
      ms: e.payload.duration_ms ?? 0,
    };
    readerSession.sentence = e.payload.text;
    readerSession.index = index;
  });
};
