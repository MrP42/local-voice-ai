import type { Span } from "./sentences";

/**
 * Ruhige Textzeile über der Hörleiste (lva, Feinschliff Hörbuch-Sync): statt
 * jeden (Halb-)Satz einzeln einzublenden, steht ein Abschnitt aus mehreren
 * Sätzen still da; nur die Markierung des gehörten Satzes wandert. Erst wenn
 * das Hören das Ende des Abschnitts erreicht, wird umgeblättert. Rückwärts
 * springt die Anzeige nur nach einem Sprung im Hörbuch oder wenn die Stelle
 * länger klar davor liegt – kleine Schwankungen der Zeitachse bleiben unsichtbar.
 * Reine Funktionen, ohne DOM.
 */

/** Ein Satz oder – bei sehr langen Sätzen – ein Teil davon. */
export type Unit = Span;

export interface Passage {
  /** Absatz, aus dem der Abschnitt stammt. */
  para: number;
  /** Einheiten des Abschnitts (Zeichenstellen im Buch). */
  units: Unit[];
  /** Index der gerade gehörten Einheit in `units`. */
  cur: number;
  /** Erste Einheit des Abschnitts im Absatz (für Schlüssel/Übergang). */
  from: number;
  /** Seit wann die Stelle ohne Sprung vor der Anzeige liegt (ms), sonst null. */
  behindSince: number | null;
}

/** So lange muss die Stelle ohne Sprung zurückliegen, bevor die Anzeige folgt. */
export const HOLD_BACK_MS = 1500;

/**
 * Sätze in Einheiten zerlegen, die in `budget` Zeichen passen. Lange Sätze
 * werden bevorzugt nach Satzzeichen (, ; : –), sonst an Wortgrenzen geteilt.
 */
export function unitsOf(
  text: string,
  sentences: Span[],
  budget: number,
): Unit[] {
  const max = Math.max(40, Math.floor(budget));
  const out: Unit[] = [];
  for (const s of sentences) {
    let a = s.start;
    while (s.end - a > max) {
      const limit = a + max;
      const part = text.slice(a, limit);
      // Bevorzugt nach Satzzeichen in der zweiten Hälfte, sonst Wortgrenze.
      let cut = -1;
      const re = /[,;:–—)]\s/g;
      for (let m = re.exec(part); m; m = re.exec(part))
        if (m.index >= max / 2) cut = m.index + 1;
      if (cut < 0) {
        const space = part.lastIndexOf(" ");
        cut = space > max / 3 ? space : max;
      }
      let b = a + cut;
      out.push({ start: a, end: b });
      while (b < s.end && /\s/.test(text[b])) b++;
      a = b;
    }
    if (s.end > a) out.push({ start: a, end: s.end });
  }
  return out;
}

/** Einheit, in der `chars` liegt (sonst die letzte davor, sonst die erste). */
export function unitAt(units: Unit[], chars: number): number {
  let lo = 0;
  let hi = units.length - 1;
  let found = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (units[mid].start <= chars) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/** Ab Einheit `from` so viele Einheiten, wie in `budget` Zeichen passen (mind. eine). */
export function takeUnits(units: Unit[], from: number, budget: number): Unit[] {
  const out: Unit[] = [];
  let used = 0;
  for (let i = from; i < units.length; i++) {
    const len = units[i].end - units[i].start + 1;
    if (out.length && used + len > budget) break;
    out.push(units[i]);
    used += len;
  }
  return out;
}

const open = (
  para: number,
  units: Unit[],
  k: number,
  budget: number,
): Passage => ({
  para,
  units: takeUnits(units, k, budget),
  cur: 0,
  from: k,
  behindSince: null,
});

/**
 * Nächster Zustand der Zeile.
 * - `units`: Einheiten des Absatzes `para`, in dem die Stelle `chars` liegt.
 * - `seek`: seit dem letzten Aufruf wurde im Hörbuch gesprungen.
 * - `now`: Uhrzeit in ms (für das Zurückhalten kleiner Rücksprünge).
 */
export function stepPassage(
  prev: Passage | null,
  input: {
    para: number;
    units: Unit[];
    chars: number;
    budget: number;
    seek: boolean;
    now: number;
  },
): Passage | null {
  const { para, units, chars, budget, seek, now } = input;
  if (!units.length) return null;
  const k = unitAt(units, chars);
  if (!prev) return open(para, units, k, budget);
  if (seek) {
    // Sprung innerhalb des sichtbaren Abschnitts: Text bleibt stehen.
    const inside =
      para === prev.para && k >= prev.from && k < prev.from + prev.units.length;
    return inside
      ? { ...prev, cur: k - prev.from, behindSince: null }
      : open(para, units, k, budget);
  }

  // Position relativ zur Anzeige: -1 davor, 0 im Abschnitt, 1 dahinter.
  let where: -1 | 0 | 1;
  let idx = -1;
  if (para !== prev.para) where = para < prev.para ? -1 : 1;
  else if (k < prev.from) where = -1;
  else if (k >= prev.from + prev.units.length) where = 1;
  else {
    where = 0;
    idx = k - prev.from;
  }

  if (where === 1) return open(para, units, k, budget);
  if (where === 0 && idx >= prev.cur)
    return idx === prev.cur && prev.behindSince === null
      ? prev
      : { ...prev, cur: idx, behindSince: null };
  // Stelle liegt vor der Anzeige: erst nach HOLD_BACK_MS folgen.
  const since = prev.behindSince ?? now;
  if (now - since < HOLD_BACK_MS)
    return prev.behindSince === since ? prev : { ...prev, behindSince: since };
  return where === 0
    ? { ...prev, cur: idx, behindSince: null }
    : open(para, units, k, budget);
}

/** Abschnitt neu ab der gehörten Einheit (z. B. wenn sie nicht mehr sichtbar ist). */
export function restartAtCurrent(
  p: Passage,
  units: Unit[],
  budget: number,
): Passage {
  return open(p.para, units, p.from + p.cur, budget);
}
