// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
/** Rückmeldung des Backends nach dem Vorwärmen. */
export interface CallReady {
  llm: string;
  llm_local: boolean;
  warnings: string[];
  warmup_ms: number;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

/** Teilergebnis einer laufenden Erkennung (Diktat). */
export interface PartialTranscript {
  /** Bisher erkannter Text der laufenden Äußerung (ersetzt das vorige Teilergebnis). */
  text: string;
  /** true: Text ist endgültig, die Äußerung ist abgeschlossen. */
  final: boolean;
}

/** Laufende Erkennung mit Teilergebnissen (ADR-0021, 5.). */
export interface TranscribeStream {
  /** PCM16, 16 kHz, mono, little endian als Base64; Rahmen in Aufnahmereihenfolge. */
  push(pcm16B64: string): Promise<void>;
  /** Eingabe beendet; löst mit dem endgültigen Text auf. */
  finish(): Promise<string>;
  /** Abbrechen, Teilergebnisse verwerfen. */
  cancel(): Promise<void>;
}

/**
 * Korrektursignal aus dem Diktat (ADR-0021, 7. und Codex-Auflage 3): nur als
 * Verhörer erkannte Wortpaare, nur mit Einwilligung, immer mit Bereich.
 */
export interface DictationFeedback {
  /** Wissensbereich des Chats; Profile werden je Bereich getrennt gehalten. */
  bereich: string;
  pairs: { heard: string; meant: string }[];
}

/**
 * Hören und Sprechen. Das liefert der Sprachdienst (Local Voice AI) jeder App;
 * Denken (`chat`) gehört nicht dazu (ADR-0021, 5.).
 */
export interface SpeechIO {
  /** Modelle vorwärmen, Hinweise liefern. */
  start(): Promise<CallReady>;
  /** PCM16, 16 kHz, mono, little endian als Base64 -> Text. */
  transcribe(pcm16B64: string): Promise<string>;
  /** Erkennung mit Teilergebnissen; fehlt sie, nutzt das Diktat `transcribe` je Äußerung. */
  transcribeStream?(onPartial: (p: PartialTranscript) => void): Promise<TranscribeStream>;
  /** Ein Häppchen -> WAV als Base64. */
  synthesize(text: string): Promise<string>;
  /** Laufende Erkennung/Antwort/Synthese abbrechen (darf vor dem Start eintreffen). */
  cancel(requestId: string): Promise<void>;
  /** Korrektursignal (Opt-in); fehlt es, lernt der Dienst nicht aus Korrekturen. */
  feedback?(f: DictationFeedback): Promise<void>;
  /** Aufräumen (Listener, Verbindungen). */
  dispose?(): void;
}

/**
 * Was ein Gesprächspartner braucht: Hören und Sprechen plus Denken. Die App
 * liefert die Umsetzung; in Corporate LLM kommt `chat` aus der App selbst und
 * `SpeechIO` aus dem Sprachdienst, in Local Voice AI beides aus dem eigenen Kern.
 */
export interface SpeechBackend extends SpeechIO {
  /**
   * Antwort des Sprachmodells als Strom sprechbarer Häppchen. `onSentence`
   * wird je Häppchen gerufen; die Rückgabe löst erst auf, wenn das letzte
   * geliefert ist, und enthält den ganzen Text.
   */
  chat(
    requestId: string,
    messages: ChatMessage[],
    onSentence: (text: string) => void,
  ): Promise<string>;
}

/** Denken aus der App, wenn Hören/Sprechen aus dem Sprachdienst kommen. */
export interface ChatSide {
  chat: SpeechBackend["chat"];
  /** Laufende Antwort abbrechen. */
  cancel?: (requestId: string) => Promise<void>;
  dispose?: () => void;
}

/**
 * Fügt Hören/Sprechen und Denken aus zwei Quellen zu einem `SpeechBackend` zusammen.
 * `cancel` ist bestmöglich: Es bricht beide Seiten ab und scheitert nur, wenn beide scheitern.
 * Die Zuordnung `requestId` → Strom des Sprachdienstes führt die `SpeechIO`-Umsetzung.
 */
export function combineBackend(io: SpeechIO, side: ChatSide): SpeechBackend {
  const stream = io.transcribeStream;
  const feedback = io.feedback;
  return {
    start: () => io.start(),
    transcribe: (b) => io.transcribe(b),
    ...(stream
      ? { transcribeStream: (cb: (p: PartialTranscript) => void) => stream.call(io, cb) }
      : {}),
    synthesize: (t) => io.synthesize(t),
    ...(feedback ? { feedback: (f: DictationFeedback) => feedback.call(io, f) } : {}),
    chat: side.chat,
    cancel: async (id) => {
      const parts = [io.cancel(id), side.cancel ? side.cancel(id) : Promise.resolve()];
      const results = await Promise.allSettled(parts);
      if (results.every((r) => r.status === "rejected")) {
        throw (results[0] as PromiseRejectedResult).reason;
      }
    },
    dispose: () => {
      io.dispose?.();
      side.dispose?.();
    },
  };
}
