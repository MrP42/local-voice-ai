import type { Erreicht, ProtokollStand } from "@/bindings";

/** Die Werte von „Ziel erreicht“; die Bewertung setzt ein Mensch, nie ein Modell. */
export const ERREICHT_WERTE: readonly Erreicht[] = [
  "ja",
  "teilweise",
  "nein",
] as const;

/** Fehlercodes der Protokoll-Befehle der Planung, die einen eigenen Text haben. */
export const PROTOKOLL_FEHLER = [
  "plan_nicht_gefunden",
  "besprechung_fehlt",
  "protokoll_fehlt",
  "protokoll_nicht_freigegeben",
  "protokoll_nicht_aktualisiert",
] as const;

/**
 * Der Text eines abgefangenen Fehlers: `api` wirft `Error(<code>)`, darum zählt die Meldung, nicht
 * `String(e)` (das den Präfix „Error: “ voranstellt und den Code unkenntlich machte).
 */
export const fehlerMeldung = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

/** Der Code einer Fehlermeldung (`"<code>"` oder `"<code>: <detail>"`). */
export const fehlerCode = (roh: string): string => {
  const text = roh.replace(/^Error:\s*/, "");
  const i = text.indexOf(":");
  return (i < 0 ? text : text.slice(0, i)).trim();
};

/** Schlüssel des Fehlertexts; Unbekanntes gilt als allgemeiner Fehler. */
export const protokollFehlerSchluessel = (roh: string): string => {
  const code = fehlerCode(roh);
  return (PROTOKOLL_FEHLER as readonly string[]).includes(code)
    ? `planung.protokoll.fehler.${code}`
    : "planung.protokoll.fehler.allgemein";
};

/** Was die Oberfläche anbietet, abgeleitet allein aus dem Stand des Backends. */
export interface Angebot {
  /** „Protokoll aus Agenda“: es gibt eine Besprechung, aus der sich ein Protokoll erzeugen lässt. */
  erzeugen: boolean;
  /** „Protokoll freigeben“: ein Protokoll liegt vor und ist (noch) nicht in dieser Fassung freigegeben. */
  freigeben: boolean;
  /** „Entwurf an Teilnehmende“: nur aus der freigegebenen, seither unveränderten Fassung. */
  entwurf: boolean;
}

export const angebot = (stand: ProtokollStand | null): Angebot => ({
  erzeugen: !!stand?.meeting_id,
  freigeben: !!stand?.dokument_id && !stand.freigegeben,
  entwurf: !!stand?.freigegeben,
});
