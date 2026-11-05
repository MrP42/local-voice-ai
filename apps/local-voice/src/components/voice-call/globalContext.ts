/**
 * Globaler Sprachassistent (lva#325, Plan P3): Kontext aus allen
 * freigegebenen Inhalten der App für einen Gesprächsschritt.
 *
 * Ablauf je Schritt: nach der Spracherkennung (auch beim spekulativen Start
 * ab `early_end`) holt `prepare` lokal die besten Auszüge
 * (`app_context_retrieve`) und baut die Kontextnachricht
 * (`app_context_message`). Kein Modell-, TTS- oder Cloud-Aufruf geschieht hier
 * (Plan M5). Das Ergebnis ist an Schritt, Transkript und Indexstand gebunden;
 * `discard` verwirft es bei `resume`, Abbruch oder Seitenwechsel.
 *
 * Die Kontextnachricht steht direkt VOR der neuen Frage, nach Systemprompt und
 * Verlauf: der vordere Teil bleibt gleich und kann im Prompt-Cache lokaler
 * Modelle liegen (Plan L3).
 *
 * Rein bis auf das injizierte `call` (Tauri `invoke`), damit
 * `node --experimental-strip-types --test` es ohne App prüft.
 */

export type SourceKind =
  | "book"
  | "book_note"
  | "ki_doc"
  | "meeting"
  | "dictation"
  | "conversation"
  | "tts_page";

export interface Jump {
  kind: SourceKind;
  id: string;
  offset: number;
  page: number | null;
  book_id: string | null;
}

export interface ContextHit {
  k: number;
  kind: SourceKind;
  title: string;
  chapter: string | null;
  at_ms: number;
  excerpt: string;
  jump: Jump;
  score: number;
  version: string;
}

export interface ContextHits {
  hits: ContextHit[];
  searched: SourceKind[];
  retrieve_ms: number;
  epoch: number;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

/** Vorbereiteter Kontext eines Schritts. */
export interface PreparedContext {
  turnId: string;
  transcript: string;
  hits: ContextHits;
  /** `null` ohne Treffer: dann geht keine Kontextnachricht an das Modell. */
  message: string | null;
  /** Dauer bis der Prompt-Teil fertig war (ms, Messung t2/t3). */
  retrieveMs: number;
  promptMs: number;
}

export type Call = <T>(
  cmd: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export interface PrepareInput {
  turnId: string;
  transcript: string;
  previous?: string | null;
  conversationId?: string | null;
  wholeBooks?: string[];
  german?: boolean;
}

/** Fügt die Kontextnachricht direkt vor die letzte Nutzernachricht ein. */
export function withContext(
  messages: ChatMessage[],
  context: string | null,
): ChatMessage[] {
  if (!context) return messages;
  let last = -1;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "user") {
      last = i;
      break;
    }
  }
  const at = last < 0 ? messages.length : last;
  return [
    ...messages.slice(0, at),
    { role: "system", content: context },
    ...messages.slice(at),
  ];
}

const MARKER = /\s*\[Q\d+\]/g;

/** Belegmarken `[Qk]` für die Stimme entfernen (angezeigt, nicht gesprochen). */
export function speakable(sentence: string): string {
  return sentence.replace(MARKER, "");
}

/** Verwaltet den Kontext laufender Schritte. */
export class GlobalContext {
  private readonly call: Call;
  private readonly now: () => number;
  private pending = new Map<string, Promise<PreparedContext | null>>();
  private keys = new Map<string, string>();
  /** Zuletzt gesehener Indexstand: ein neuerer Stand macht ältere Ergebnisse ungültig. */
  private epoch = -1;

  constructor(call: Call, now: () => number = () => performance.now()) {
    this.call = call;
    this.now = now;
  }

  /**
   * Kontext für einen Schritt holen (oder den schon laufenden mit gleichem
   * Transkript wiederverwenden). Fehler ergeben `null`: das Gespräch läuft
   * dann ohne Kontext weiter, statt zu hängen.
   */
  prepare(input: PrepareInput): Promise<PreparedContext | null> {
    const key = `${input.turnId}\u001f${input.transcript}`;
    const existing = this.pending.get(input.turnId);
    if (existing && this.keys.get(input.turnId) === key) return existing;
    const started = this.now();
    const p = (async (): Promise<PreparedContext | null> => {
      try {
        const hits = await this.call<ContextHits>("app_context_retrieve", {
          req: {
            question: input.transcript,
            previous: input.previous ?? null,
            conversation_id: input.conversationId ?? null,
            whole_books: input.wholeBooks ?? [],
          },
        });
        const retrieved = this.now();
        if (hits.epoch > this.epoch) this.epoch = hits.epoch;
        const message = hits.hits.length
          ? await this.call<string | null>("app_context_message", {
              hits,
              german: input.german ?? true,
            })
          : null;
        return {
          turnId: input.turnId,
          transcript: input.transcript,
          hits,
          message: message ?? null,
          retrieveMs: Math.round(retrieved - started),
          promptMs: Math.round(this.now() - started),
        };
      } catch {
        return null;
      }
    })();
    this.pending.set(input.turnId, p);
    this.keys.set(input.turnId, key);
    return p;
  }

  /**
   * Das gültige Ergebnis für einen Schritt: nur, wenn Transkript und
   * Indexstand noch stimmen; sonst `null` (dann neu holen).
   */
  async take(
    turnId: string,
    transcript: string,
  ): Promise<PreparedContext | null> {
    const p = this.pending.get(turnId);
    if (!p || this.keys.get(turnId) !== `${turnId}\u001f${transcript}`)
      return null;
    const ctx = await p;
    if (!ctx || ctx.hits.epoch < this.epoch) return null;
    return ctx;
  }

  /** Spekulation verwerfen (resume, Abbruch, Seitenwechsel). */
  discard(turnId: string): void {
    this.pending.delete(turnId);
    this.keys.delete(turnId);
  }

  /** Alles verwerfen (Schalter aus, Freigaben geändert, Gespräch beendet). */
  clear(): void {
    this.pending.clear();
    this.keys.clear();
  }

  /**
   * Belege einer fertigen Antwort prüfen: nur Marken, die es in diesem Abruf
   * gab, mit unveränderter Quelle und aktuellem Stand. Fehler: keine Belege.
   */
  async verify(answer: string, ctx: PreparedContext): Promise<ContextHit[]> {
    try {
      const ok = await this.call<number[]>("app_context_verify", {
        answer,
        hits: ctx.hits,
      });
      const valid = new Set(ok);
      return ctx.hits.hits.filter((h) => valid.has(h.k));
    } catch {
      return [];
    }
  }
}
