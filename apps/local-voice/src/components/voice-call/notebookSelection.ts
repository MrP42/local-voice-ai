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
