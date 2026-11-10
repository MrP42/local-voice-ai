import {
  ALL_AREAS,
  GLOBAL_AREAS,
  KNOWLEDGE_AREAS,
  NOTEBOOK_AREA,
} from "./bereiche";

/**
 * „Zugriff auf alle App-Inhalte einschalten“ (lva#485): ein Schritt statt vier Schalter. Reine Logik
 * ohne Oberfläche; die Einstellungen kommen über `Zugriff` herein, damit sie prüfbar bleibt.
 */

/** Die drei Einstellungen, die den Zugriff bestimmen (Hauptschalter zuletzt). */
export const ZUGRIFF_SCHLUESSEL = [
  "voice_call_knowledge",
  "voice_call_read_transcripts",
  "voice_global_context_enabled",
] as const;
export type ZugriffSchluessel = (typeof ZUGRIFF_SCHLUESSEL)[number];

type ZugriffWert = string[] | boolean;

export interface Zugriff {
  lies(key: ZugriffSchluessel): ZugriffWert | null | undefined;
  /** Speichert eine Einstellung. Wirft nicht: scheitert es, rollt der Speicher den Wert zurück. */
  setze(key: ZugriffSchluessel, value: ZugriffWert): Promise<void>;
}

/**
 * Alle Bereiche der Bereichsliste, ohne „Notizbuch“: dessen Abschnitte sind eine eigene Freigabe.
 * Ist das Notizbuch schon freigegeben, bleibt es (nichts, was die Person gewählt hat, geht verloren).
 */
export function alleBereiche(aktuell: readonly string[]): string[] {
  const ohneNotizbuch: readonly string[] = [
    ...KNOWLEDGE_AREAS,
    ...GLOBAL_AREAS,
  ];
  return ALL_AREAS.filter(
    (a) =>
      ohneNotizbuch.includes(a) ||
      (a === NOTEBOOK_AREA && aktuell.includes(NOTEBOOK_AREA)),
  );
}

/** Ist schon alles frei (Hauptschalter, alle Bereiche ohne Notizbuch, Transkripte)? */
export function alleFrei(stand: {
  enabled: boolean;
  areas: readonly string[];
  transcripts: boolean;
}): boolean {
  return (
    stand.enabled &&
    stand.transcripts &&
    [...KNOWLEDGE_AREAS, ...GLOBAL_AREAS].every((a) => stand.areas.includes(a))
  );
}

function gleich(a: unknown, b: ZugriffWert): boolean {
  if (typeof b === "boolean") return a === b;
  return (
    Array.isArray(a) && a.length === b.length && a.every((x, i) => x === b[i])
  );
}

/**
 * Schaltet den Zugriff ein: erst die Bereiche und die Transkripte, zuletzt der Hauptschalter (bricht
 * etwas ab, wirkt der Zugriff nie halb). Der Speicher wirft bei einem Fehler nicht, er rollt den
 * Schlüssel zurück; deshalb zählt nach jedem Schritt der gelesene Wert. Scheitert ein Schritt, gehen
 * die schon geschriebenen Schlüssel auf ihren alten Wert zurück. `true`: alles ist eingeschaltet.
 */
export async function schalteAllesEin(io: Zugriff): Promise<boolean> {
  const alt = io.lies("voice_call_knowledge");
  const ziel: Record<ZugriffSchluessel, ZugriffWert> = {
    voice_call_knowledge: alleBereiche(Array.isArray(alt) ? alt : []),
    voice_call_read_transcripts: true,
    voice_global_context_enabled: true,
  };
  const vorher = new Map(ZUGRIFF_SCHLUESSEL.map((k) => [k, io.lies(k)]));
  const geschrieben: ZugriffSchluessel[] = [];
  for (const key of ZUGRIFF_SCHLUESSEL) {
    if (gleich(vorher.get(key), ziel[key])) continue;
    await io.setze(key, ziel[key]);
    if (gleich(io.lies(key), ziel[key])) {
      geschrieben.push(key);
      continue;
    }
    for (const done of geschrieben.reverse()) {
      const frueher = vorher.get(done);
      await io.setze(
        done,
        done === "voice_call_knowledge"
          ? Array.isArray(frueher)
            ? frueher
            : []
          : frueher === true,
      );
    }
    return false;
  }
  return true;
}
