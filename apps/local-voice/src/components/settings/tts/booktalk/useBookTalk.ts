import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import type {
  BookTalkInput,
  BookTalkRequest,
  BookTalkScope,
} from "@/bindings";
import type { BookTalkPort } from "./port";
import {
  fromStored,
  lastCitation,
  reduceTurns,
  runningTurn,
  type Turn,
} from "./turns";

let counter = 0;
const newRequestId = () => `bt-${Date.now().toString(36)}-${counter++}`;

export interface BookTalkState {
  turns: Turn[];
  /** Bereich der nächsten Frage; „Ganzes Buch“ gilt, bis die Spalte schließt. */
  scope: BookTalkScope;
  busy: boolean;
  /** Geladen (Verlauf gelesen)? Vorher zeigt die Spalte keine Vorschläge. */
  ready: boolean;
  ask(question: string, input: BookTalkInput): Promise<void>;
  /** Frage stellen, die schon läuft (Sprache über die Buch-Seite): nur in den Verlauf nehmen. */
  adopt(req: BookTalkRequest): void;
  setScope(scope: BookTalkScope): void;
  /** Laufende Frage abbrechen (Esc, Stopp). */
  cancel(): void;
  /** Dieselbe Frage im aktuellen Bereich noch einmal („Im neuen Bereich fragen“). */
  askAgain(turn: Turn): void;
  clear(): Promise<void>;
  /** Beleg für „Spring dahin“ und „Lies die Stelle vor“. */
  lastCitation(): ReturnType<typeof lastCitation>;
  /** Aktueller Bereich, auch aus Rückrufen heraus (die Engine hält keine React-Werte). */
  getScope(): BookTalkScope;
}

/**
 * Gesprächsspalte, Zustand und Anfragen (lva#309). Eine Unterhaltung je Buch
 * (Plan D1): Verlauf aus dem Kern, Sprache und Text im selben Strom. Bereich
 * und Grenze gelten je Anfrage unveränderlich (§3.7); ein Bereichswechsel
 * während einer Antwort bricht sie ab, die Frage wird nicht automatisch neu
 * gestellt.
 */
export function useBookTalk(opts: {
  port: BookTalkPort;
  bookId: string | null;
  /** Lesegrenze zum Zeitpunkt der Frage; `undefined` = unbekannt. */
  frontier(): number | undefined;
}): BookTalkState {
  const { port, bookId } = opts;
  const [turns, dispatch] = useReducer(reduceTurns, [] as Turn[]);
  const [scope, setScopeState] = useState<BookTalkScope>("until_position");
  const [ready, setReady] = useState(false);
  const turnsRef = useRef<Turn[]>(turns);
  turnsRef.current = turns;
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const frontierRef = useRef(opts.frontier);
  frontierRef.current = opts.frontier;

  // Anderes Buch oder Spalte zu: Bereich fällt auf „bis hier“ zurück, Verlauf lädt neu.
  useEffect(() => {
    setScopeState("until_position");
    setReady(false);
    dispatch({ type: "clear" });
    if (!bookId) return;
    let alive = true;
    void port
      .history(bookId)
      .then((stored) => {
        if (!alive) return;
        dispatch({ type: "load", turns: fromStored(stored) });
        setReady(true);
      })
      .catch(() => alive && setReady(true));
    return () => {
      alive = false;
      const running = runningTurn(turnsRef.current);
      if (running) void port.cancel(running.id).catch(() => false);
    };
  }, [port, bookId]);

  // Ereignisse des Kerns: nur Anfragen dieses Verlaufs zählen (der Reducer filtert).
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    void port.subscribe((event) => dispatch({ type: "event", event })).then(
      (unlisten) => {
        if (alive) off = unlisten;
        else unlisten();
      },
    );
    return () => {
      alive = false;
      off?.();
    };
  }, [port]);

  const adopt = useCallback((req: BookTalkRequest) => {
    dispatch({
      type: "begin",
      turn: {
        id: req.request_id,
        question: req.question,
        input: req.input,
        scope: req.scope,
        frontier: req.frontier ?? 0,
      },
    });
  }, []);

  const ask = useCallback(
    async (question: string, input: BookTalkInput) => {
      const clean = question.trim();
      if (!clean || !bookId || runningTurn(turnsRef.current)) return;
      const frontier = frontierRef.current();
      const req: BookTalkRequest = {
        book_id: bookId,
        request_id: newRequestId(),
        question: clean,
        ...(frontier !== undefined ? { frontier } : {}),
        scope: scopeRef.current,
        input,
      };
      adopt(req);
      try {
        const answer = await port.ask(req);
        dispatch({ type: "result", id: req.request_id, answer });
      } catch (e) {
        const code = e instanceof Error ? e.message : String(e);
        dispatch({ type: "error", id: req.request_id, code });
      }
    },
    [port, bookId, adopt],
  );

  const cancelRunning = useCallback(
    (reason: "scope" | "user") => {
      const running = runningTurn(turnsRef.current);
      if (!running) return;
      dispatch({ type: "cancel", id: running.id, reason });
      void port.cancel(running.id).catch(() => false);
    },
    [port],
  );

  const setScope = useCallback(
    (next: BookTalkScope) => {
      if (next === scopeRef.current) return;
      cancelRunning("scope");
      scopeRef.current = next;
      setScopeState(next);
    },
    [cancelRunning],
  );

  const askAgain = useCallback(
    (turn: Turn) => void ask(turn.question, turn.input),
    [ask],
  );

  const clear = useCallback(async () => {
    if (!bookId) return;
    cancelRunning("user");
    await port.clear(bookId);
    dispatch({ type: "clear" });
  }, [port, bookId, cancelRunning]);

  return {
    turns,
    scope,
    busy: runningTurn(turns) !== undefined,
    ready,
    ask,
    adopt,
    setScope,
    cancel: () => cancelRunning("user"),
    askAgain,
    clear,
    lastCitation: () => lastCitation(turnsRef.current),
    getScope: () => scopeRef.current,
  };
}
