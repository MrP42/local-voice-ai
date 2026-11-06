/**
 * Sprachwechsel mit Positionsübernahme (lva#344, Spezifikation §6).
 *
 * Eine Fassung wird als Gliederung beschrieben: Kapitelanfänge in ihrer
 * Einheit (Zeichen beim Text, Millisekunden beim Hörbuch) und Gesamtlänge.
 * Zwischen zwei Fassungen gilt:
 * - **kapitelgenau** nur, wenn das Quellkapitel in einer bestätigten Gruppe
 *   der Kapitelzuordnung liegt: Anteil innerhalb der Gruppe → Zielgruppe;
 * - sonst **ungefähr**: Anteil am ganzen Buch.
 * „Genau“ (gleiche Fassung, Text ↔ Hörbuch) regelt die Zeitachse der Fassung
 * und ist nicht Teil dieser Funktion.
 */

export interface Outline {
  /** Kapitelanfänge, aufsteigend, in der Einheit der Fassung. */
  starts: number[];
  /** Gesamtlänge in derselben Einheit. */
  length: number;
}

export interface AlignGroup {
  a: number[];
  b: number[];
  confirmed: boolean;
}

export type SwitchLevel = "chapter" | "approx";

export interface SwitchTarget {
  offset: number;
  level: SwitchLevel;
  /** Zielkapitel (0-basiert) der Gruppe bzw. das Kapitel an der Zielstelle. */
  chapters: number[];
  /** Anteil am ganzen Buch (0…1), für „ungefähr bei 34 %“. */
  fraction: number;
}

const clamp = (x: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, x));

/** Kapitel (0-basiert), in dem `offset` liegt; vor dem ersten: 0. */
export function chapterOf(outline: Outline, offset: number): number {
  let found = 0;
  for (let i = 0; i < outline.starts.length; i++) {
    if (outline.starts[i] <= offset) found = i;
    else break;
  }
  return found;
}

/** Bereich [von, bis) der Kapitel `first`…`last`. */
function span(outline: Outline, first: number, last: number): [number, number] {
  const from = outline.starts[first] ?? 0;
  const to = outline.starts[last + 1] ?? outline.length;
  return [from, Math.max(from, to)];
}

/** Gruppen lückenlos, zusammenhängend, jedes Kapitel genau einmal (wie Rust). */
export function validGroups(groups: AlignGroup[]): boolean {
  let na = 0;
  let nb = 0;
  for (const g of groups) {
    if (!g.a.length || !g.b.length) return false;
    if (g.a.some((c, i) => c !== na + i)) return false;
    if (g.b.some((c, i) => c !== nb + i)) return false;
    na += g.a.length;
    nb += g.b.length;
  }
  return true;
}

/** „Alle bestätigen“ ist nur bei gleicher Kapitelzahl und reinen 1:1-Gruppen erlaubt. */
export function canConfirmAll(
  groups: AlignGroup[],
  countA: number,
  countB: number,
): boolean {
  return (
    countA === countB &&
    groups.length === countA &&
    groups.every((g) => g.a.length === 1 && g.b.length === 1)
  );
}

/** Vorschlag: gleiche Kapitelzahl → 1:1, sonst eine Gruppe je Kapitel von `a`
 *  mit proportional verteilten Kapiteln von `b`. Nichts ist bestätigt. */
export function proposeGroups(countA: number, countB: number): AlignGroup[] {
  if (countA <= 0 || countB <= 0) return [];
  const groups: AlignGroup[] = [];
  let nb = 0;
  for (let i = 0; i < countA; i++) {
    const end =
      i === countA - 1
        ? countB
        : Math.max(nb + 1, Math.round(((i + 1) * countB) / countA));
    const room = countB - (countA - 1 - i); // je folgendem a-Kapitel mindestens ein b
    const last = Math.min(end, Math.max(nb + 1, room));
    if (nb >= countB) {
      // Mehr a- als b-Kapitel: an die vorige Gruppe anhängen (n:1).
      groups[groups.length - 1].a.push(i);
      continue;
    }
    const b: number[] = [];
    for (let c = nb; c < last; c++) b.push(c);
    groups.push({ a: [i], b, confirmed: false });
    nb = last;
  }
  return groups;
}

/**
 * Zielstelle in Fassung `to` für Stelle `offset` in Fassung `from`.
 * `groups` beschreibt die Zuordnung von `from` (a) nach `to` (b); ist sie in
 * der anderen Richtung gespeichert, vorher mit {@link flipGroups} drehen.
 */
export function mapPosition(
  from: Outline,
  to: Outline,
  offset: number,
  groups: AlignGroup[] | null,
): SwitchTarget {
  const fraction = from.length > 0 ? clamp(offset / from.length, 0, 1) : 0;
  const approx = (): SwitchTarget => {
    const target = Math.round(fraction * to.length);
    return {
      offset: target,
      level: "approx",
      chapters: to.starts.length ? [chapterOf(to, target)] : [],
      fraction,
    };
  };
  if (
    !groups ||
    !validGroups(groups) ||
    !from.starts.length ||
    !to.starts.length
  )
    return approx();
  const ch = chapterOf(from, offset);
  const g = groups.find((x) => x.a.includes(ch));
  if (!g || !g.confirmed) return approx();
  const lastB = g.b[g.b.length - 1];
  if (lastB >= to.starts.length) return approx();
  const [fa, ta] = span(from, g.a[0], g.a[g.a.length - 1]);
  const [fb, tb] = span(to, g.b[0], lastB);
  const within = ta > fa ? clamp((offset - fa) / (ta - fa), 0, 1) : 0;
  const target = Math.round(fb + within * (tb - fb));
  return {
    offset: target,
    level: "chapter",
    chapters: g.b,
    fraction: to.length > 0 ? clamp(target / to.length, 0, 1) : fraction,
  };
}

export const flipGroups = (groups: AlignGroup[]): AlignGroup[] =>
  groups.map((g) => ({ a: g.b, b: g.a, confirmed: g.confirmed }));
