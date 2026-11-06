import type {
  BookTalkAnswer,
  BookTalkCommandKind,
  BookTalkEvent,
  BookTalkRequest,
  BookTalkScope,
} from "@/bindings";
import type { BookTalkPort } from "@/components/settings/tts/booktalk/port";
import type { ChatSide } from "@/vendor/wai-voice/backend.ts";

/**
 * „Buch“-Seite des Sprachgesprächs (lva#309, Plan §3.7, G3): Die Engine hört,
 * erkennt Sprache und spricht (Turn-Taking, Barge-in); das Denken kommt hier
 * aus dem Kern `book_talk_ask`. Jeder geprüfte Satz geht mit seiner
 * Sprechfassung sofort an die Stimme (`onSentence`), ohne die ganze Antwort
 * abzuwarten. Sprachbefehle („Spring dahin“, „Weiterlesen“, „Ganzes Buch“)
 * laufen über `book_talk_command` und rufen das Modell nicht.
 *
 * Bei Abbruch (Bereichswechsel, Esc, neue Frage) ruft die Seite
 * `book_talk_cancel`, verwirft alle späteren Ereignisse der alten `request_id`
 * und liefert nichts mehr an die Stimme; die Engine leert ihre Warteschlange.
 */

/** Wartezeit bis zum Füllsatz, wenn noch kein inhaltlicher Satz da ist (ms). */
export const FILLER_MS = 600;
export const FILLER_DE = "Moment, ich schaue im Buch nach.";
export const FILLER_EN = "One moment, I am checking the book.";
export const SCOPE_ASK_DE = "Auch spätere Kapitel einbeziehen? Sag Ja.";
export const SCOPE_ASK_EN = "Include later chapters too? Say yes.";

export interface BookSideContext {
  bookId: string;
  scope(): BookTalkScope;
  /** Lesegrenze (UTF-16); `undefined`, solange sie unbekannt ist. */
  frontier(): number | undefined;
  /** Eine Anfrage beginnt: die Spalte nimmt die Frage in den Verlauf. */
  onRequest(req: BookTalkRequest): void;
  /**
   * Ein Sprachbefehl soll ausgeführt werden (`scope_book` erst nach dem
   * gesprochenen „Ja“). Ein Rückgabetext wird gesprochen.
   */
  onCommand(kind: BookTalkCommandKind): Promise<string | void> | string | void;
}

export interface BookSideTimers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface BookSideOptions {
  fillerMs?: number;
  timers?: BookSideTimers;
}

export interface BookSide extends ChatSide {
  /** Kennung der laufenden Anfrage, falls eine läuft. */
  activeRequestId(): string | null;
  /** Laufende Anfrage abbrechen (Bereichswechsel, Esc). */
  cancelActive(): Promise<void>;
}

const YES = new Set([
  "ja",
  "jo",
  "jawohl",
  "gerne",
  "klar",
  "ja gerne",
  "ja klar",
  "yes",
  "yeah",
  "sure",
  "okay",
  "yes please",
]);

const normalize = (text: string): string =>
  text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();

const isEnglish = (text: string): boolean =>
  /\b(book|chapters?|spoilers?|please|only|whole|entire|read|what|where|who|the)\b/i.test(
    text,
  );

interface Active {
  id: string;
  cancelled: boolean;
  wake: () => void;
  /**
   * Ist erst erfüllt, wenn der Aufruf `port.ask` dieser Anfrage zurück ist (der
   * Kern gibt den Lauf-Wächter dann frei); eine Nachfolgerin wartet darauf,
   * sonst meldet der Kern `chat_busy`.
   */
  settled: Promise<void>;
}

export function createBookSide(
  port: BookTalkPort,
  ctx: BookSideContext,
  options: BookSideOptions = {},
): BookSide {
  const fillerMs = options.fillerMs ?? FILLER_MS;
  const timers: BookSideTimers = options.timers ?? {
    set: (fn, ms) => setTimeout(fn, ms),
    clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  let active: Active | null = null;
  let awaitingScope = false;

  const cancelRun = async (run: Active): Promise<void> => {
    if (run.cancelled) return;
    run.cancelled = true;
    run.wake();
    await port.cancel(run.id).catch(() => false);
  };

  const runCommand = async (
    kind: BookTalkCommandKind,
    question: string,
    onSentence: (text: string) => void,
  ): Promise<string> => {
    if (kind === "scope_book") {
      awaitingScope = true;
      const ask = isEnglish(question) ? SCOPE_ASK_EN : SCOPE_ASK_DE;
      onSentence(ask);
      return ask;
    }
    awaitingScope = false;
    const said = (await ctx.onCommand(kind)) ?? "";
    if (said) onSentence(said);
    return said;
  };

  const ask = async (
    requestId: string,
    question: string,
    onSentence: (text: string) => void,
  ): Promise<string> => {
    // Eine neue Frage ersetzt eine noch laufende: erst die alte abbrechen und
    // ihren Aufruf abwarten (der Kern lässt nur eine Antwort zugleich zu).
    if (active) {
      const previous = active;
      await cancelRun(previous);
      await previous.settled;
    }
    // Lauf anmelden, bevor irgendetwas wartet: ein Abbruch während der
    // Anmeldung am Ereignisstrom trifft so schon diesen Lauf.
    let release: () => void = () => {};
    const run: Active = {
      id: requestId,
      cancelled: false,
      wake: () => {},
      settled: new Promise<void>((resolve) => {
        release = resolve;
      }),
    };
    const woken = new Promise<{ cancelled: true }>((resolve) => {
      run.wake = () => resolve({ cancelled: true });
    });
    active = run;
    const delivered = new Map<number, string>();
    const spoken: string[] = [];
    let filler: unknown = null;
    let off: (() => void) | null = null;
    let asking = false;
    const stopFiller = () => {
      if (filler === null) return;
      timers.clear(filler);
      filler = null;
    };
    const say = (text: string) => {
      if (run.cancelled || !text.trim()) return;
      spoken.push(text);
      onSentence(text);
    };
    const handle = (e: BookTalkEvent) => {
      if (run.cancelled || e.request_id !== requestId) return;
      if (e.kind !== "sentence") return;
      if (delivered.has(e.sentence.index)) return;
      stopFiller();
      delivered.set(e.sentence.index, e.sentence.speech);
      say(e.sentence.speech);
    };
    try {
      off = await port.subscribe(handle);
      // Während der Anmeldung abgebrochen: abmelden und nichts fragen.
      if (run.cancelled) return spoken.join(" ");
      // Bereich und Grenze erst jetzt: sie gelten für genau diese Anfrage.
      const frontier = ctx.frontier();
      const req: BookTalkRequest = {
        book_id: ctx.bookId,
        request_id: requestId,
        question,
        ...(frontier !== undefined ? { frontier } : {}),
        scope: ctx.scope(),
        input: "voice",
      };
      ctx.onRequest(req);
      filler = timers.set(() => {
        filler = null;
        if (run.cancelled || spoken.length > 0) return;
        say(isEnglish(question) ? FILLER_EN : FILLER_DE);
      }, fillerMs);
      asking = true;
      const asked = port.ask(req).then(
        (answer) => ({ answer }),
        (error: unknown) => ({ error }),
      );
      void asked.then(release, release);
      const outcome = await Promise.race([asked, woken]);
      stopFiller();
      if ("cancelled" in outcome || run.cancelled) return spoken.join(" ");
      if ("error" in outcome) {
        if (String(outcome.error).includes("cancelled")) return spoken.join(" ");
        throw outcome.error instanceof Error
          ? outcome.error
          : new Error(String(outcome.error));
      }
      deliverRest(outcome.answer, delivered, say);
      return spoken.join(" ");
    } finally {
      stopFiller();
      off?.();
      // Wurde `port.ask` gestartet, gibt erst dessen Rückkehr die Nachfolgerin frei.
      if (!asking) release();
      if (active === run) active = null;
    }
  };

  return {
    async chat(requestId, messages, onSentence) {
      const question =
        [...messages].reverse().find((m) => m.role === "user")?.content.trim() ??
        "";
      if (!question) return "";
      // Die Antwort auf die Rückfrage „Auch spätere Kapitel einbeziehen?“ gilt
      // zuerst: ein „Ja“ ist hier die Bestätigung, kein anderer Befehl.
      if (awaitingScope && YES.has(normalize(question))) {
        awaitingScope = false;
        return confirmScope(ctx, onSentence);
      }
      const command = await port.command(ctx.bookId, question).catch(() => null);
      if (command) return runCommand(command.kind, question, onSentence);
      awaitingScope = false;
      return ask(requestId, question, onSentence);
    },
    async cancel(requestId) {
      if (active && active.id === requestId) await cancelRun(active);
    },
    async cancelActive() {
      if (active) await cancelRun(active);
    },
    activeRequestId: () => (active && !active.cancelled ? active.id : null),
    dispose() {
      if (active) void cancelRun(active);
      awaitingScope = false;
    },
  };
}

async function confirmScope(
  ctx: BookSideContext,
  onSentence: (text: string) => void,
): Promise<string> {
  const said = (await ctx.onCommand("scope_book")) ?? "";
  if (said) onSentence(said);
  return said;
}

/**
 * Was die Ereignisse nicht geliefert haben: fehlende Sätze (Ereignis nach dem
 * Rückgabewert) und das Ende der Sprechfassung (Rückfrage „Soll ich dir die
 * Stelle vorlesen?“ bzw. „bis Seite N“), das nicht zu einem Satz gehört.
 */
function deliverRest(
  answer: BookTalkAnswer,
  delivered: Map<number, string>,
  say: (text: string) => void,
): void {
  for (const s of answer.sentences)
    if (!delivered.has(s.index)) {
      delivered.set(s.index, s.speech);
      say(s.speech);
    }
  const joined = [...delivered.entries()]
    .sort(([a], [b]) => a - b)
    .map(([, speech]) => speech)
    .join(" ");
  if (!answer.speech.startsWith(joined)) return;
  const tail = answer.speech.slice(joined.length).trim();
  if (tail) say(tail);
}
