// Firmenrichtlinie (lva#882): welche Schalter wirken, und wie eine vom Backend
// abgelehnte Aktion verständlich angezeigt wird. Reine Logik ohne React, damit
// `scripts/firmenrichtlinie.test.mjs` sie direkt prüfen kann.
import type { RichtlinienAusgabe } from "../bindings";

export type Schalter =
  | "keine_programm_downloads"
  | "kein_selbstupdate"
  | "kein_autostart"
  | "keine_tastatur_hooks"
  | "keine_besprechungserkennung"
  | "kein_systemton";

export const SCHALTER: readonly Schalter[] = [
  "keine_programm_downloads",
  "kein_selbstupdate",
  "kein_autostart",
  "keine_tastatur_hooks",
  "keine_besprechungserkennung",
  "kein_systemton",
];

const istSchalter = (name: string): name is Schalter =>
  (SCHALTER as readonly string[]).includes(name);

/**
 * Gesperrt ist ein Schalter nur, wenn die Richtlinie ihn setzt UND die App ihn
 * tatsächlich durchsetzt. Ein gesetzter, aber (noch) nicht durchgesetzter
 * Schalter würde sonst eine Einschränkung anzeigen, die es nicht gibt.
 */
export const istGesperrt = (
  ausgabe: RichtlinienAusgabe | null | undefined,
  schalter: Schalter,
): boolean =>
  !!ausgabe &&
  ausgabe[schalter] === true &&
  ausgabe.durchgesetzt.includes(schalter);

/** Alle wirksam gesperrten Schalter, in fester Reihenfolge. */
export const gesperrteSchalter = (
  ausgabe: RichtlinienAusgabe | null | undefined,
): Schalter[] => SCHALTER.filter((schalter) => istGesperrt(ausgabe, schalter));

/**
 * Das Backend meldet Sperren als `richtlinie:<schalter>: <Text>`. Der Text kann
 * in einer längeren Fehlerkette stehen (z. B. „Download fehlgeschlagen: …“),
 * deshalb wird überall gesucht, nicht nur am Anfang.
 */
export const richtlinienSchalter = (fehler: unknown): Schalter | null => {
  const text =
    fehler instanceof Error
      ? fehler.message
      : typeof fehler === "string"
        ? fehler
        : "";
  const treffer = /richtlinie:([a-z_]+):/.exec(text);
  return treffer && istSchalter(treffer[1]) ? treffer[1] : null;
};

/** Übersetzungsschlüssel der Meldung zu einer Sperre. */
export const meldungsSchluessel = (schalter: Schalter): string =>
  `firmenrichtlinie.gesperrt.${schalter}`;

/** Seite, auf der es neue Versionen gibt, wenn die App sich nicht selbst aktualisiert. */
export const RELEASE_SEITE =
  "https://github.com/MrP42/local-voice-ai/releases/latest";

/**
 * Verständliche Meldung zu einem Fehler, wenn er von der Firmenrichtlinie
 * stammt; sonst `null`, und der Aufrufer zeigt seine bisherige Meldung.
 */
export const richtlinienText = (
  t: (schluessel: string) => string,
  fehler: unknown,
): string | null => {
  const schalter = richtlinienSchalter(fehler);
  return schalter ? t(meldungsSchluessel(schalter)) : null;
};
