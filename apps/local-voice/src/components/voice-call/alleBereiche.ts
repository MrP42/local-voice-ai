import {
  ALL_AREAS,
  GLOBAL_AREAS,
  KNOWLEDGE_AREAS,
  NOTEBOOK_AREA,
} from "./bereiche";

/**
 * „Bücher, Wissensbereiche und Transkripte freigeben“ (lva#485, atomar seit lva#509): ein Schritt
 * statt vier Schalter. Reine Logik ohne Oberfläche; der Kern kommt über `Zugriff` herein, damit sie
 * prüfbar bleibt.
 *
 * Freigeben ist EIN Aufruf an den Kern, der unter einer Sperre die erwartete Berechtigungsversion
 * vergleicht und Hauptschalter, Bereiche und Transkripte zusammen schreibt. Hat sich der Zugriff
 * inzwischen geändert (etwa ein Widerruf), schreibt der Kern nichts und meldet den Konflikt; ein
 * Zurückrollen gibt es nicht mehr, ein Widerruf kann also nie überschrieben werden.
 */

/** Stand des Inhaltszugriffs samt Berechtigungsversion (`voice_call_inhaltszugriff_lesen`). */
export interface Inhaltsstand {
  version: number;
  enabled: boolean;
  areas: string[];
  read_transcripts: boolean;
}

/** Antwort von `voice_call_inhaltszugriff_setzen`. */
export interface SetzenAntwort {
  status: "ok" | "konflikt";
  version: number;
}

export interface Zugriff {
  lies(): Promise<Inhaltsstand>;
  /** Wirft, wenn der Kern einen Fehler meldet; ein Konflikt ist eine Antwort, kein Fehler. */
  setze(
    erwarteteVersion: number,
    enabled: boolean,
    areas: string[],
    readTranscripts: boolean,
  ): Promise<SetzenAntwort>;
  /** Liest den Stand der Oberfläche neu ein (nach Erfolg, Konflikt und Fehler). */
  uebernehmen(): Promise<void>;
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

/**
 * `ein`: alles freigegeben · `konflikt`: der Zugriff wurde inzwischen geändert, nichts geschrieben ·
 * `fehler`: Lesen oder Setzen schlug fehl, nichts geschrieben.
 */
export type Ergebnis = "ein" | "konflikt" | "fehler";

/**
 * Gibt den Zugriff frei: Version lesen, dann EIN Setzen mit dieser Version. Danach wird der Stand der
 * Oberfläche in jedem Fall neu gelesen (auch nach Konflikt oder Fehler), damit sie zeigt, was gilt.
 */
export async function schalteAllesEin(io: Zugriff): Promise<Ergebnis> {
  try {
    const stand = await io.lies();
    const antwort = await io.setze(
      stand.version,
      true,
      alleBereiche(stand.areas),
      true,
    );
    return antwort.status === "ok" ? "ein" : "konflikt";
  } catch {
    return "fehler";
  } finally {
    try {
      await io.uebernehmen();
    } catch {
      /* die Anzeige bleibt, wie sie war; der Kern hat den Stand */
    }
  }
}

/**
 * Wohin der Fokus nach der Freigabe geht. Die Taste verschwindet bei Erfolg (an ihrer Stelle steht
 * die Statusanzeige): wer sie per Tastatur ausgelöst hat, bekommt den Fokus auf die Statusanzeige
 * (`tabIndex=-1`), damit er nicht auf `body` fällt und die Meldung vorgelesen wird. Bei Konflikt oder
 * Fehler bleibt die Taste; sie bekommt den Fokus zurück. Lag der Fokus nicht auf der Taste, bleibt er.
 */
export function fokusNachFreigabe(
  ergebnis: Ergebnis,
  tasteHatteFokus: boolean,
): "status" | "taste" | "keiner" {
  if (!tasteHatteFokus) return "keiner";
  return ergebnis === "ein" ? "status" : "taste";
}
