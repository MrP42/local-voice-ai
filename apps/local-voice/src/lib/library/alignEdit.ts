/**
 * Kapitelzuordnung bearbeiten (lva#344, Spezifikation §6.1). Jede Änderung
 * liefert eine neue, vollständig gültige Gruppenliste (lückenlos, in
 * Reihenfolge, jedes Kapitel genau einmal); betroffene Gruppen sind danach
 * wieder unbestätigt. Bestätigen ändert nur das Häkchen.
 */
import type { AlignGroup } from "./languageSwitch";

const open = (a: number[], b: number[]): AlignGroup => ({
  a,
  b,
  confirmed: false,
});

export type Side = "a" | "b";

/** Gruppe bestätigen. */
export const confirmGroup = (groups: AlignGroup[], i: number): AlignGroup[] =>
  groups.map((g, k) => (k === i ? { ...g, confirmed: true } : g));

/** Alle bestätigen; erlaubt nur bei gleicher Kapitelzahl und lauter 1:1-Gruppen. */
export const confirmAll = (groups: AlignGroup[]): AlignGroup[] =>
  groups.map((g) => ({ ...g, confirmed: true }));

/** Gruppe teilen: je Seite das erste Kapitel bleibt, der Rest bildet die nächste Gruppe. */
export function canSplit(g: AlignGroup): boolean {
  return g.a.length >= 2 && g.b.length >= 2;
}

export function splitGroup(groups: AlignGroup[], i: number): AlignGroup[] {
  const g = groups[i];
  if (!g || !canSplit(g)) return groups;
  return [
    ...groups.slice(0, i),
    open([g.a[0]], [g.b[0]]),
    open(g.a.slice(1), g.b.slice(1)),
    ...groups.slice(i + 1),
  ];
}

/** Mit der nächsten Gruppe verbinden. */
export function joinWithNext(groups: AlignGroup[], i: number): AlignGroup[] {
  const [g, n] = [groups[i], groups[i + 1]];
  if (!g || !n) return groups;
  return [
    ...groups.slice(0, i),
    open([...g.a, ...n.a], [...g.b, ...n.b]),
    ...groups.slice(i + 2),
  ];
}

/**
 * Grenze zwischen Gruppe `i` und der nächsten auf einer Seite verschieben:
 * `forward` gibt das letzte Kapitel von `i` an die nächste Gruppe ab, sonst
 * holt `i` das erste Kapitel der nächsten. Jede Gruppe behält mindestens ein Kapitel.
 */
export function canMoveBoundary(
  groups: AlignGroup[],
  i: number,
  side: Side,
  forward: boolean,
): boolean {
  const [g, n] = [groups[i], groups[i + 1]];
  if (!g || !n) return false;
  return (forward ? g[side] : n[side]).length >= 2;
}

export function moveBoundary(
  groups: AlignGroup[],
  i: number,
  side: Side,
  forward: boolean,
): AlignGroup[] {
  if (!canMoveBoundary(groups, i, side, forward)) return groups;
  const [g, n] = [groups[i], groups[i + 1]];
  const [gs, ns] = [g[side], n[side]];
  const [keepG, keepN] = forward
    ? [gs.slice(0, -1), [gs[gs.length - 1], ...ns]]
    : [[...gs, ns[0]], ns.slice(1)];
  const other: Side = side === "a" ? "b" : "a";
  const make = (mine: number[], theirs: number[]): AlignGroup =>
    side === "a" ? open(mine, theirs) : open(theirs, mine);
  return [
    ...groups.slice(0, i),
    make(keepG, g[other]),
    make(keepN, n[other]),
    ...groups.slice(i + 2),
  ];
}

/** Anzahl bestätigter Gruppen. */
export const confirmedCount = (groups: AlignGroup[]): number =>
  groups.filter((g) => g.confirmed).length;
