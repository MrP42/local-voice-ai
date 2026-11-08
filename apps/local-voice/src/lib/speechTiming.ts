/**
 * Welche Textstelle eines Satzes gerade zu hören ist.
 *
 * Der Kern meldet mit `tts-current-sentence` die Dauer des Satzes und die
 * stimmhaften Abschnitte darin (`speech_ms`, ms bei Tempo 1). Die Dauer
 * enthält Vorlauf, Nachklang und Absatzpause; wer die Zeichen gleichmäßig auf
 * sie verteilt, markiert gegen Satzende um genau diese Stille zu spät
 * (Patrick 08.11.). Verteilt wird deshalb nur über die Sprechzeit.
 *
 * Teilsätze sind eine HEURISTIK: Passt die Zahl der Abschnitte zu den
 * Teilsätzen (Komma, Semikolon, Doppelpunkt, Gedankenstrich), bekommt jeder
 * Teilsatz seinen Abschnitt, und die Kommapause hält die Markierung am
 * Kommawort fest. Gleiche Anzahl beweist keine richtige Zuordnung: eine
 * Atempause ohne Komma und ein Komma ohne hörbare Pause können sich
 * ausgleichen, dann steht die Markierung in beiden Teilsätzen versetzt, bis
 * der nächste Satz sie neu ansetzt. Belastbare Grenzen hätte nur die
 * Synthese selbst (Wortzeitstempel); die liefern die Anbieter heute nicht.
 * Passt die Anzahl nicht, gilt die Sprechzeit als Ganzes.
 *
 * Die Uhr ({@link SentenceClock}) läuft ab `at` mit dem Tempo und beginnt
 * bei `base`: Der Kern meldet jedes weitere Stück eines Satzes (nach einem
 * Pausen-Tag, der Absatzpause) mit seinem echten Start, und ein Tempowechsel
 * setzt sie an der erreichten Stelle neu an. So zählt weder das Öffnen des
 * Ausgabestroms zwischen zwei Stücken als gespielt, noch springt die
 * Markierung bei einem Tempowechsel.
 */

export type SpeechSpan = readonly [number, number];

export interface TextRange {
  start: number;
  end: number;
}

/** Hinter diesen Zeichen (gefolgt von Leerraum) macht eine Stimme eine Pause. */
const PAUSE_AFTER = /[,;:–—…]/;

/**
 * Gültige, aufsteigende Abschnitte innerhalb von `0..durationMs`; ist einer
 * davon Unsinn, gilt keiner (lieber gleichmäßig als falsch).
 */
export function cleanSpans(
  spans: unknown,
  durationMs: number = Infinity,
): SpeechSpan[] {
  if (!Array.isArray(spans)) return [];
  const out: SpeechSpan[] = [];
  let last = 0;
  for (const span of spans) {
    if (!Array.isArray(span) || span.length < 2) return [];
    const [a, b] = span;
    if (
      typeof a !== "number" ||
      typeof b !== "number" ||
      !Number.isFinite(a) ||
      !Number.isFinite(b) ||
      b <= a ||
      a < last ||
      b > durationMs
    )
      return [];
    out.push([a, b]);
    last = b;
  }
  return out;
}

/** Teilsätze von `range`: Grenze hinter einem Pausenzeichen vor Leerraum. */
export function clauses(text: string, range: TextRange): TextRange[] {
  const out: TextRange[] = [];
  let start = range.start;
  for (let i = range.start; i < range.end - 1; i++) {
    if (!PAUSE_AFTER.test(text[i]) || !/\s/.test(text[i + 1])) continue;
    const cut = i + 1;
    if (cut > start) out.push({ start, end: cut });
    // Der nächste Teilsatz beginnt mit seinem ersten Wort, nicht im Leerraum.
    start = cut;
    while (start < range.end && /\s/.test(text[start])) start++;
    i = start - 1;
  }
  if (range.end > start) out.push({ start, end: range.end });
  return out;
}

/**
 * Textoffset in `range`, der nach `elapsedMs` (Tempo 1, ab Satzbeginn) zu
 * hören ist. Vor dem ersten Laut der Satzanfang, in einer Pause das Ende des
 * zuvor gesprochenen Stücks, nach dem letzten Laut das Satzende.
 */
export function spokenOffset(
  text: string,
  range: TextRange,
  spans: readonly SpeechSpan[],
  elapsedMs: number,
): number {
  const length = range.end - range.start;
  if (length <= 0 || spans.length === 0) return range.start;
  const parts = clauses(text, range);
  const lastChar = range.end - 1;
  if (parts.length === spans.length && parts.length > 1) {
    for (let i = spans.length - 1; i >= 0; i--) {
      const [a, b] = spans[i];
      if (elapsedMs < a) continue;
      const part = parts[i];
      const share = Math.min(1, (elapsedMs - a) / (b - a));
      const at = part.start + Math.floor(share * (part.end - part.start));
      return Math.min(at, part.end - 1, lastChar);
    }
    return range.start;
  }
  let spoken = 0;
  let total = 0;
  for (const [a, b] of spans) {
    total += b - a;
    spoken += Math.max(0, Math.min(elapsedMs, b) - a);
  }
  if (total <= 0) return range.start;
  const at = range.start + Math.floor((spoken / total) * length);
  return Math.min(at, lastChar);
}

/** Uhr des hörbaren Satzes (alle Zeiten ms; Satzzeit bei Tempo 1). */
export interface SentenceClock {
  index: number | null;
  /** `performance.now()`, ab dem die Uhr läuft. */
  at: number;
  /** Satzzeit, die bei `at` schon gespielt war (Stückbeginn, Tempowechsel). */
  base?: number;
  /** Dauer des Satzes samt Pausen. */
  ms: number;
  speed: number;
  /** Stimmhafte Abschnitte, sofern der Kern sie meldet. */
  speech?: readonly SpeechSpan[];
  /** Beginn jedes Stücks im Satz, aufsteigend (erstes 0). */
  pieces?: readonly number[];
  /**
   * Beginn des zuletzt vom Kern als spielend gemeldeten Stücks (0 = erstes).
   * Rückt NUR mit `tts-sentence-piece` vor ({@link startPiece}), nie mit der
   * Zeitbasis: ein Tempowechsel während des Haltens gibt nichts frei.
   */
  piece?: number;
}

/**
 * Bis wohin die Uhr laufen darf: bis zum Beginn des nächsten Stücks. Das
 * Stück danach öffnet den Ausgabestrom neu und ist erst gespielt, wenn der
 * Kern es meldet (`tts-sentence-piece`) -- so lange steht die Uhr.
 */
function clockLimit(clock: SentenceClock): number {
  const released = clock.piece ?? 0;
  for (const start of clock.pieces ?? [])
    if (start > released) return Math.min(start, clock.ms);
  return clock.ms;
}

/** Gespielte Satzzeit bei Tempo 1 (0..ms). */
export function clockElapsed(clock: SentenceClock, now: number): number {
  const elapsed =
    (clock.base ?? 0) + (now - clock.at) * Math.max(0.25, clock.speed);
  return Math.max(0, Math.min(clockLimit(clock), elapsed));
}

/** Gültige, aufsteigende Stückanfänge in `0..durationMs`, sonst keine. */
export function cleanPieces(
  pieces: unknown,
  durationMs: number = Infinity,
): number[] {
  if (!Array.isArray(pieces)) return [];
  let last = -1;
  for (const p of pieces)
    if (
      typeof p !== "number" ||
      !Number.isFinite(p) ||
      p < last ||
      p < 0 ||
      p > durationMs
    )
      return [];
    else last = p;
  return pieces as number[];
}

/** Anteil (0..1) des Satzes, der schon zu hören war. */
export function clockFraction(clock: SentenceClock, now: number): number {
  return clock.ms > 0 ? clockElapsed(clock, now) / clock.ms : 0;
}

/** Die Uhr ab `now` bei `base` weiterlaufen lassen (gibt kein Stück frei). */
export function anchorClock<T extends { at: number; base?: number }>(
  clock: T,
  now: number,
  base: number,
): T {
  return { ...clock, at: now, base: Math.max(0, base) };
}

/** Der Kern meldet: das Stück ab `offset` spielt jetzt. Gibt es frei. */
export function startPiece<
  T extends { at: number; base?: number; piece?: number },
>(clock: T, now: number, offset: number): T {
  return { ...anchorClock(clock, now, offset), piece: Math.max(0, offset) };
}

/**
 * Tempowechsel mitten im Satz: das bis jetzt Gespielte mit dem ALTEN Tempo
 * festhalten, ab jetzt läuft die Uhr mit dem neuen -- kein Sprung.
 */
export function retimeClock<T extends Omit<SentenceClock, "speed">>(
  clock: T,
  now: number,
  oldSpeed: number,
): T {
  return anchorClock(
    clock,
    now,
    clockElapsed({ ...clock, speed: oldSpeed }, now),
  );
}
