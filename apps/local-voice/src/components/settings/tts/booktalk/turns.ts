import type {
  BookTalkAnswer,
  BookTalkCitation,
  BookTalkEvent,
  BookTalkInput,
  BookTalkScope,
  BookTalkSentence,
  StoredTurn,
} from "@/bindings";

/**
 * Zustand der Gesprächsspalte (lva#309): je Frage ein Wechsel. Rein, damit die
 * Regeln aus Plan §3.7 prüfbar sind: nur die laufende `request_id` zählt,
 * Ereignisse einer abgebrochenen oder älteren Anfrage werden verworfen.
 */

export type TurnStatus = "pending" | "done" | "cancelled" | "failed";

/** Warum ein Wechsel abgebrochen wurde: Bereichswechsel oder Wunsch des Nutzers. */
export type CancelReason = "scope" | "user";

export interface Turn {
  /** `request_id` der Anfrage (bei gespeicherten Wechseln `stored-<n>`). */
  id: string;
  question: string;
  input: BookTalkInput;
  scope: BookTalkScope;
  frontier: number;
  sentences: BookTalkSentence[];
  status: TurnStatus;
  answer?: BookTalkAnswer;
  cancelReason?: CancelReason;
  /** Fehlercode, wenn der Aufruf selbst scheiterte (kein Ereignis, keine Antwort). */
  error?: string;
}

export interface NewTurn {
  id: string;
  question: string;
  input: BookTalkInput;
  scope: BookTalkScope;
  frontier: number;
}

export type TurnAction =
  | { type: "load"; turns: Turn[] }
  | { type: "begin"; turn: NewTurn }
  | { type: "event"; event: BookTalkEvent }
  | { type: "result"; id: string; answer: BookTalkAnswer }
  | { type: "error"; id: string; code: string }
  | { type: "cancel"; id: string; reason: CancelReason }
  | { type: "clear" };

export function fromStored(stored: StoredTurn[]): Turn[] {
  return stored.map((s, i) => ({
    id: `stored-${i}`,
    question: s.question,
    input: s.input,
    scope: s.scope,
    frontier: s.frontier,
    sentences: s.answer.sentences,
    status: "done",
    answer: s.answer,
  }));
}

const update = (
  turns: Turn[],
  id: string,
  change: (t: Turn) => Turn,
): Turn[] => {
  const i = turns.findIndex((t) => t.id === id);
  if (i < 0) return turns;
  const next = turns.slice();
  next[i] = change(turns[i]);
  return next;
};

const running = (t: Turn) => t.status === "pending";

export function reduceTurns(turns: Turn[], action: TurnAction): Turn[] {
  switch (action.type) {
    case "load":
      return action.turns;
    case "clear":
      return [];
    case "begin":
      return [
        ...turns,
        { ...action.turn, sentences: [], status: "pending" as const },
      ];
    case "event": {
      const e = action.event;
      return update(turns, e.request_id, (t) => {
        if (!running(t)) return t;
        if (e.kind === "sentence") {
          // Gleicher Index ersetzt (Wiederholung), sonst anhängen.
          const rest = t.sentences.filter((s) => s.index !== e.sentence.index);
          return { ...t, sentences: [...rest, e.sentence].sort(byIndex) };
        }
        if (e.kind === "done")
          return {
            ...t,
            status: "done",
            answer: e.answer,
            sentences: e.answer.sentences,
          };
        return e.code === "cancelled"
          ? { ...t, status: "cancelled", cancelReason: t.cancelReason ?? "user" }
          : { ...t, status: "failed", error: e.code };
      });
    }
    case "result":
      // Rückgabewert des Befehls: gilt nur, wenn das `done`-Ereignis noch fehlt.
      return update(turns, action.id, (t) =>
        running(t)
          ? {
              ...t,
              status: "done",
              answer: action.answer,
              sentences: action.answer.sentences,
            }
          : t,
      );
    case "error":
      return update(turns, action.id, (t) =>
        running(t)
          ? action.code === "cancelled"
            ? {
                ...t,
                status: "cancelled",
                cancelReason: t.cancelReason ?? "user",
              }
            : { ...t, status: "failed", error: action.code }
          : t,
      );
    case "cancel":
      return update(turns, action.id, (t) =>
        running(t)
          ? { ...t, status: "cancelled", cancelReason: action.reason }
          : t,
      );
  }
}

const byIndex = (a: BookTalkSentence, b: BookTalkSentence) => a.index - b.index;

/** Die laufende Anfrage (höchstens eine). */
export const runningTurn = (turns: Turn[]): Turn | undefined =>
  turns.find(running);

/** Alle Belege eines Wechsels in Sprechreihenfolge, ohne Doppelte (gleicher Satz). */
export function citationsOf(t: Turn): BookTalkCitation[] {
  const seen = new Set<string>();
  const out: BookTalkCitation[] = [];
  const all = [
    ...t.sentences.flatMap((s) => s.citations),
    ...(t.answer?.citations ?? []),
  ];
  for (const c of all) {
    const key = `${c.start}:${c.end}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(c);
  }
  return out;
}

/** Letzter Beleg des letzten fertigen Wechsels: Ziel für „Spring dahin“ und „Lies die Stelle vor“. */
export function lastCitation(turns: Turn[]): BookTalkCitation | null {
  for (let i = turns.length - 1; i >= 0; i--) {
    const t = turns[i];
    if (t.status !== "done") continue;
    const cites = citationsOf(t);
    if (cites.length > 0) return cites[0];
  }
  return null;
}
