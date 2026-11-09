/**
 * Auswahl der Notizbuch-Freigabe (lva#396 N2): reine Umrechnung zwischen der Einstellung
 * `voice_call_notebook_sections` (`abschnitt:<id>`, `notizbuch:<id>`) und der Anzeige. Unbekannte
 * Einträge (etwa Abschnitte, die die Gliederung gerade nicht zeigt) bleiben beim Speichern erhalten.
 */

import type { NotebookOutline } from "@/bindings";

export interface Auswahl {
  abschnitte: Set<string>;
  /** Ganze Notizbücher samt künftiger Abschnitte. */
  ganz: Set<string>;
}

const ABSCHNITT = "abschnitt:";
const NOTIZBUCH = "notizbuch:";

export function auswahlAus(eintraege: readonly string[]): Auswahl {
  const a: Auswahl = { abschnitte: new Set(), ganz: new Set() };
  for (const e of eintraege) {
    if (e.startsWith(ABSCHNITT)) a.abschnitte.add(e.slice(ABSCHNITT.length));
    else if (e.startsWith(NOTIZBUCH)) a.ganz.add(e.slice(NOTIZBUCH.length));
  }
  return a;
}

/** Einträge zur Auswahl; die Reihenfolge der bisherigen Einträge bleibt, neue kommen hinten an. */
export function eintraegeAus(
  a: Auswahl,
  bisher: readonly string[] = [],
): string[] {
  const soll = new Set([
    ...[...a.ganz].map((id) => NOTIZBUCH + id),
    ...[...a.abschnitte].map((id) => ABSCHNITT + id),
  ]);
  const aus = bisher.filter((e) => soll.has(e));
  for (const e of soll) if (!aus.includes(e)) aus.push(e);
  return aus;
}

export function setzeAbschnitte(
  a: Auswahl,
  ids: readonly string[],
  an: boolean,
): Auswahl {
  const abschnitte = new Set(a.abschnitte);
  for (const id of ids) {
    if (an) abschnitte.add(id);
    else abschnitte.delete(id);
  }
  return { abschnitte, ganz: new Set(a.ganz) };
}

export function setzeGanz(a: Auswahl, nb: string, an: boolean): Auswahl {
  const ganz = new Set(a.ganz);
  if (an) ganz.add(nb);
  else ganz.delete(nb);
  return { abschnitte: new Set(a.abschnitte), ganz };
}

/** Freigegebene Abschnitte eines Notizbuchs (ganz = alle). */
export function frei(a: Auswahl, nb: NotebookOutline): number {
  if (a.ganz.has(nb.id)) return nb.sections.length;
  return nb.sections.filter((s) => a.abschnitte.has(s.id)).length;
}

/**
 * Anfangs aufgeklappte Notizbücher: die mit Auswahl, ohne jede Auswahl das erste. Wird einmal beim
 * ersten Anzeigen bestimmt; Änderungen der Auswahl klappen danach nichts mehr zu oder auf (sonst
 * verschwände das fokussierte Kästchen, Codex lva#418).
 */
export function startOffen(
  nbs: readonly NotebookOutline[],
  a: Auswahl,
): Record<string, boolean> {
  const mitWahl = nbs.filter((n) => frei(a, n) > 0 || a.ganz.has(n.id));
  const offen: Record<string, boolean> = {};
  for (const n of mitWahl) offen[n.id] = true;
  if (!mitWahl.length && nbs[0]) offen[nbs[0].id] = true;
  return offen;
}

/**
 * Schreibt Werte nacheinander: nie zwei Schreibvorgänge zugleich, und von mehreren Änderungen
 * während eines Schreibens geht nur die letzte hinterher. So kann keine ältere Antwort eine neuere
 * Auswahl überholen. `fertig` meldet das Ende einer Kette mit dem Ergebnis des letzten Schreibens.
 */
export function schreibkette<T>(
  schreiben: (v: T) => Promise<boolean>,
  fertig: (ok: boolean) => void,
): { setze: (v: T) => void; laeuft: () => boolean } {
  let laeuft = false;
  let naechster: { v: T } | null = null;
  const lauf = async (v: T) => {
    laeuft = true;
    let ok = false;
    try {
      ok = await schreiben(v);
    } catch {
      ok = false;
    }
    if (naechster) {
      const n = naechster;
      naechster = null;
      return lauf(n.v);
    }
    laeuft = false;
    fertig(ok);
  };
  return {
    setze: (v) => {
      if (laeuft) naechster = { v };
      else void lauf(v);
    },
    laeuft: () => laeuft,
  };
}
