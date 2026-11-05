import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "@/i18n";
import type { BookTalkCommandKind, BookTalkScope } from "@/bindings";
import {
  DEFAULT_CALL_CONFIG,
  VoiceCallEngine,
  type CallState,
} from "@/components/voice-call/engine";
import { createBookSide, type BookSide } from "@/components/voice-call/bookSide";
import type { BookTalkPort } from "./port";
import type { BookTalkState } from "./useBookTalk";

/**
 * Die Stelle im Buch, auf die ein Beleg zeigt (UTF-16, Ende exklusiv).
 */
export interface BookSpot {
  start: number;
  end: number;
}

/** Was das Vorlesen im Reader dem Gespräch erlaubt (lva#309, Plan G3). */
export interface BookTalkHost {
  /** Läuft das Vorlesen gerade? */
  isReading(): boolean;
  /** Stelle des Vorlesens (Offset im Buchtext); `null`, wenn unbekannt. */
  position(): number | null;
  /** Pausieren und die Stelle merken. */
  pauseReading(): void;
  /** An der gemerkten Stelle fortsetzen („Weiterlesen“). */
  resumeReading(): void;
  /** Ab dieser Stelle vorlesen („Lies mir die Stelle vor“, „Ab hier vorlesen“). */
  readFrom(offset: number): void;
  /** Satz markieren (3 s) und aufschlagen. */
  jumpTo(spot: BookSpot): void;
}

export type VoicePhase =
  "off" | "starting" | "listening" | "hearing" | "thinking" | "speaking";

export interface BookVoice {
  phase: VoicePhase;
  /** Eingangspegel 0…1 für die Welle. */
  level: number;
  error: string | null;
  /** Vorlesen wurde für das Gespräch angehalten und wartet auf „Weiterlesen“. */
  readingPaused: boolean;
  /** Mikrofon an/aus (Sprechen-Knopf). */
  toggle(): void;
  /** Gespräch beenden; das Vorlesen setzt dort fort, wo es pausiert wurde (außer `resumeReading` ist false). */
  end(resumeReading?: boolean): void;
  /** Vorlesen an der alten Stelle fortsetzen. */
  resume(): void;
  /** Bereich wechseln und dabei Antwort, Sprechwarteschlange und Anfrage abbrechen. */
  setScope(scope: BookTalkScope): void;
  /** Laufende Antwort abbrechen (Esc, Stopp). */
  stopAnswer(): void;
}

/** Nach dem Loslassen der Leertaste so lange auf Sprache warten, sonst Ende (ms). */
export const PTT_GRACE_MS = 900;

const typing = (t: EventTarget | null): boolean =>
  t instanceof Element &&
  t.closest(
    'input, textarea, select, button, a, [contenteditable=""], [contenteditable="true"], [role="button"], [role="menuitem"], [role="dialog"]',
  ) !== null;

/**
 * Sprachgespräch mit dem Buch (lva#309, G3): eigene Engine mit der Seite
 * „Buch“ (`bookSide.ts`), Vorlesen pausiert beim Start, Sprachbefehle über
 * `book_talk_command`, Tastatur (Leertaste halten = sprechen, Esc beendet).
 */
export function useBookVoice(opts: {
  port: BookTalkPort;
  bookId: string | null;
  talk: BookTalkState;
  frontier(): number | undefined;
  host: BookTalkHost;
  /** Ohne Buch oder bei ausgeschalteter Tastatur reagiert nichts. */
  keyboard?: boolean;
}): BookVoice {
  const { port, bookId } = opts;
  const [phase, setPhase] = useState<VoicePhase>("off");
  const [level, setLevel] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [readingPaused, setReadingPaused] = useState(false);
  const live = useRef(opts);
  live.current = opts;
  const engineRef = useRef<VoiceCallEngine | null>(null);
  const sideRef = useRef<BookSide | null>(null);
  const pausedByTalk = useRef(false);
  const pttHeld = useRef(false);
  /** Die Sitzung wurde mit der Leertaste gestartet: sie endet nach einer Antwort von selbst. */
  const pttSession = useRef(false);
  const sawTurn = useRef(false);
  const pttTimer = useRef<number | null>(null);
  const phaseRef = useRef<VoicePhase>("off");

  const setPausedFlag = (paused: boolean) => {
    pausedByTalk.current = paused;
    setReadingPaused(paused);
  };

  const closeSession = useCallback(async (resumeReading: boolean) => {
    const engine = engineRef.current;
    engineRef.current = null;
    sideRef.current?.dispose?.();
    sideRef.current = null;
    pttHeld.current = false;
    pttSession.current = false;
    sawTurn.current = false;
    if (pttTimer.current !== null) window.clearTimeout(pttTimer.current);
    pttTimer.current = null;
    phaseRef.current = "off";
    setPhase("off");
    setLevel(0);
    if (engine) {
      await engine.stop().catch(() => undefined);
      await engine.dispose().catch(() => undefined);
    }
    if (pausedByTalk.current) {
      setPausedFlag(false);
      if (resumeReading) live.current.host.resumeReading();
    }
  }, []);

  /** Befehl ausführen (gerufen aus der Buch-Seite, mitten im Schritt der Engine). */
  const runCommand = useCallback(
    (kind: BookTalkCommandKind): string | void => {
      const { host, talk } = live.current;
      const later = (fn: () => void) => window.setTimeout(fn, 0);
      switch (kind) {
        case "jump": {
          const spot = talk.lastCitation();
          if (!spot) return i18n.t("tts.bookTalk.voice.say.noSpot");
          host.jumpTo(spot);
          return;
        }
        case "read_citation": {
          const spot = talk.lastCitation();
          if (!spot) return i18n.t("tts.bookTalk.voice.say.noSpot");
          later(() => {
            void closeSession(false).then(() => host.readFrom(spot.start));
          });
          return;
        }
        case "resume":
          later(() => void closeSession(true));
          return;
        case "end":
          later(() => void closeSession(true));
          return;
        case "stop":
          talk.cancel();
          void sideRef.current?.cancelActive();
          engineRef.current?.interrupt();
          return;
        case "scope_book":
          talk.setScope("book");
          return i18n.t("tts.bookTalk.voice.say.scopeBook");
        case "scope_position":
          talk.setScope("until_position");
          return i18n.t("tts.bookTalk.voice.say.scopePosition");
        case "confirm":
          return;
      }
    },
    [closeSession],
  );

  const start = useCallback(async () => {
    const { host } = live.current;
    if (!bookId || engineRef.current) return;
    setError(null);
    sawTurn.current = false;
    if (host.isReading()) {
      host.pauseReading();
      setPausedFlag(true);
    }
    const side = createBookSide(port, {
      bookId,
      scope: () => live.current.talk.getScope(),
      frontier: () => live.current.frontier(),
      onRequest: (req) => live.current.talk.adopt(req),
      onCommand: (kind) => runCommand(kind),
    });
    sideRef.current = side;
    const engine = new VoiceCallEngine(
      {
        onState: (s: CallState) => {
          phaseRef.current = s === "idle" ? "off" : s;
          setPhase(phaseRef.current);
          if (s === "thinking") sawTurn.current = true;
          // Mit der Leertaste gestartet: nach der Antwort ist das Gespräch zu Ende.
          if (
            s === "listening" &&
            pttSession.current &&
            !pttHeld.current &&
            sawTurn.current
          )
            window.setTimeout(() => void closeSession(true), 0);
        },
        onLevel: setLevel,
        onUser: () => {},
        onAssistant: () => {},
        onMetrics: () => {},
        onError: (message) => setError(message),
      },
      DEFAULT_CALL_CONFIG,
    );
    engine.setSideOverride(side);
    engineRef.current = engine;
    try {
      await engine.start();
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      if (message !== "abgebrochen") setError(message);
      await closeSession(true);
    }
  }, [bookId, port, runCommand, closeSession]);

  const toggle = useCallback(() => {
    if (engineRef.current) void closeSession(true);
    else void start();
  }, [start, closeSession]);

  const stopAnswer = useCallback(() => {
    live.current.talk.cancel();
    void sideRef.current?.cancelActive();
    engineRef.current?.interrupt();
  }, []);

  const setScope = useCallback(
    (scope: BookTalkScope) => {
      void sideRef.current?.cancelActive();
      engineRef.current?.interrupt();
      live.current.talk.setScope(scope);
    },
    [],
  );

  // Buch gewechselt oder Spalte geschlossen: Sitzung beenden, Vorlesen bleibt pausiert.
  useEffect(
    () => () => {
      void closeSession(false);
    },
    [bookId, closeSession],
  );

  // Tastatur: Leertaste halten = sprechen, Esc beendet Zuhören bzw. Antwort.
  useEffect(() => {
    if (opts.keyboard === false || !bookId) return;
    const down = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Escape") {
        if (engineRef.current) {
          e.preventDefault();
          if (live.current.talk.busy) stopAnswer();
          else void closeSession(true);
        } else if (live.current.talk.busy) {
          e.preventDefault();
          live.current.talk.cancel();
        }
        return;
      }
      if (e.key !== " " || e.repeat || typing(e.target)) return;
      e.preventDefault();
      // Mit laufender Sitzung hört das Mikrofon freihändig; die Leertaste startet nur eine.
      if (engineRef.current) return;
      pttHeld.current = true;
      pttSession.current = true;
      void start();
    };
    const up = (e: KeyboardEvent) => {
      if (e.key !== " " || !pttHeld.current) return;
      pttHeld.current = false;
      const engine = engineRef.current;
      if (!engine) return;
      // Nach dem Loslassen schweigt das Mikrofon; kommt keine Äußerung, endet die Sitzung.
      engine.setMuted(true);
      pttTimer.current = window.setTimeout(() => {
        pttTimer.current = null;
        if (phaseRef.current === "listening") void closeSession(true);
      }, PTT_GRACE_MS);
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [opts.keyboard, bookId, start, stopAnswer, closeSession]);

  const resume = useCallback(() => {
    if (engineRef.current) void closeSession(true);
    else if (pausedByTalk.current) {
      setPausedFlag(false);
      live.current.host.resumeReading();
    }
  }, [closeSession]);

  return {
    phase,
    level,
    error,
    readingPaused,
    toggle,
    end: (resumeReading = true) => void closeSession(resumeReading),
    resume,
    setScope,
    stopAnswer,
  };
}
