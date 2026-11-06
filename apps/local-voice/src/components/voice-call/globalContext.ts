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
  question_hash?: string;
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
  /** Sprache der Kontextnachricht. */
  german: boolean;
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

/** Wohin ein Beleg führt: eine Vorlesen-Seite oder ein App-Bereich. */
export type SourceTarget = { page: string } | { section: string } | null;

/**
 * Sprungziel eines Belegs. Bücher, Seiten und KI-Dokumente öffnen die Seite
 * im Vorlesen (KI-Dokumente tragen die Seite vor dem `/`), Notizen ihr Buch;
 * Besprechungen, Diktate und Gespräche ihren Bereich.
 */
export function sourceTarget(jump: Jump): SourceTarget {
  switch (jump.kind) {
    case "book":
    case "tts_page":
      return { page: jump.id };
    case "ki_doc":
      return { page: jump.book_id ?? jump.id.split("/")[0] };
    case "book_note":
      return jump.book_id ? { page: jump.book_id } : null;
    case "meeting":
      return { section: "meetings" };
    case "dictation":
      return { section: "history" };
    case "conversation":
      return { section: "voice" };
    default:
      return null;
  }
}

/** Gesprochener Wechsel im Buch-Gespräch: allgemein (ganze App) oder Buch. */
export type TalkMode = "general" | "book";

const MODE_FILLERS = new Set([
  "bitte",
  "mal",
  "doch",
  "jetzt",
  "hey",
  "wieder",
  "please",
  "now",
  "just",
]);

const MODE_FORMS: [TalkMode, boolean, string[]][] = [
  [
    "general",
    true,
    [
      "allgemein",
      "frag allgemein",
      "allgemeine frage",
      "allgemeines gespräch",
      "außerhalb des buchs",
      "außerhalb vom buch",
      "über alles",
      "in der ganzen app",
    ],
  ],
  [
    "general",
    false,
    ["general", "in general", "general question", "outside the book"],
  ],
  [
    "book",
    true,
    [
      "im buch",
      "nur im buch",
      "zurück zum buch",
      "zurück ins buch",
      "frage zum buch",
      "zum buch",
    ],
  ],
  ["book", false, ["in the book", "back to the book", "about the book"]],
];

/**
 * Ist die ganze Äußerung ein Wechselbefehl? Wie bei den Buchbefehlen zählt nur
 * die ganze Äußerung; „Was steht im Buch über X?“ ist eine Frage, kein Befehl.
 */
export function modeSwitch(
  text: string,
): { mode: TalkMode; german: boolean } | null {
  const words = text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter((w) => w && !MODE_FILLERS.has(w));
  if (words.length === 0 || words.length > 5) return null;
  const joined = words.join(" ");
  for (const [mode, german, forms] of MODE_FORMS)
    if (forms.includes(joined)) return { mode, german };
  return null;
}

/** Kurze gesprochene Bestätigung des Wechsels. */
export function modeSwitchReply(mode: TalkMode, german: boolean): string {
  if (german)
    return mode === "general"
      ? "Gut, jetzt allgemein über alle freigegebenen Inhalte."
      : "Gut, wieder nur im Buch.";
  return mode === "general"
    ? "Okay, now across all your shared content."
    : "Okay, back to the book only.";
}

const MARKER = /\s*\[Q\d+\]/g;

/** Belegmarken `[Qk]` für die Stimme entfernen (angezeigt, nicht gesprochen). */
export function speakable(sentence: string): string {
  return sentence.replace(MARKER, "");
}

/**
 * Was an `voice_call_chat` geht: nur die Treffer. Die Kontextnachricht baut
 * das Backend selbst, und nur für einen lokalen Empfänger mit aktuellem Stand
 * (Codex M5/M6). Ohne Treffer: kein Kontext.
 */
export function speakerContext(
  ctx: PreparedContext | null,
): { hits: ContextHits; german: boolean; turn_id: string } | null {
  if (!ctx || ctx.hits.hits.length === 0) return null;
  return { hits: ctx.hits, german: ctx.german, turn_id: ctx.turnId };
}

/** Verwaltet den Kontext laufender Schritte. */
export class GlobalContext {
  private readonly call: Call;
  private readonly now: () => number;
  private pending = new Map<string, Promise<PreparedContext | null>>();
  private keys = new Map<string, string>();
  /** Der laufende Abruf: ein neuer wartet auf ihn (nie zwei parallel). */
  private inflight: Promise<unknown> = Promise.resolve();
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
    // Höchstens eine Spekulation gleichzeitig (Codex Auflage 12).
    for (const id of [...this.pending.keys()])
      if (id !== input.turnId) this.discard(id);
    const previous = this.inflight;
    let p!: Promise<PreparedContext | null>;
    // Verworfen, während der Abruf lief (resume, Abbruch, neuer Schritt)?
    const stale = () => this.pending.get(input.turnId) !== p;
    p = (async (): Promise<PreparedContext | null> => {
      try {
        // Höchstens ein Abruf zur Zeit (Codex M5 Must-fix 5).
        await previous.catch(() => undefined);
        if (stale()) return null;
        const started = this.now();
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
        if (stale()) return null;
        const message = hits.hits.length
          ? await this.call<string | null>("app_context_message", {
              hits,
              german: input.german ?? true,
            })
          : null;
        if (stale()) return null;
        return {
          turnId: input.turnId,
          transcript: input.transcript,
          hits,
          message: message ?? null,
          german: input.german ?? true,
          retrieveMs: Math.round(retrieved - started),
          promptMs: Math.round(this.now() - started),
        };
      } catch {
        return null;
      }
    })();
    this.inflight = p;
    this.pending.set(input.turnId, p);
    this.keys.set(input.turnId, key);
    return p;
  }

  /**
   * Das gültige Ergebnis für einen Schritt: nur, wenn Transkript und
   * (zuletzt gesehener) Indexstand noch stimmen; sonst `null`. Verbindlich
   * prüft erst das Backend unmittelbar vor dem Modellstart (`admissible`).
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
