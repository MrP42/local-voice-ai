/**
 * Texte zu den Fehlern der Verwaltung (lva#556, G2 Teil 2): ein Grund des Speichers oder der Prüfung als Satz, und die
 * Fehlerliste des Kerns (`pfad`, `code`) als Zeilen. Nichts wird verschwiegen oder geglättet: jede Abweisung nennt das Feld.
 */
import type { TFunction } from "i18next";
import type { ProfilFehler } from "@/components/voice-call/assistenten";
import type { Fehlergrund } from "@/components/voice-call/profil/verwaltung";
import { feldVon } from "@/components/voice-call/profil/editor";

/** Der Satz zu einem Grund. */
export function grundText(t: TFunction, grund: Fehlergrund): string {
  return t(`assistenten.fehler.${grund}`);
}

const AUSSPRACHE_PFAD = /^aussprache(?:\[(\d+)\](?:\.(wort|aussprache))?)?$/;

/** Der Name eines Feldes für die Anzeige; Pfade, die der Editor nicht kennt, bleiben, wie sie sind. */
export function pfadText(t: TFunction, pfad: string): string {
  if (pfad === "") return t("assistenten.pfad.datei");
  const bekannt = [
    "schema",
    "id",
    "name",
    "beschreibung",
    "anweisungen",
    "begruessung",
    "sprache",
    "stimme",
    "verhalten.stil",
    "verhalten.sprechpause",
    "verhalten.unterbrechen",
    "verhalten.nachfrage_stille_s",
  ];
  // Punkte im Pfad sind in den Übersetzungen Unterstriche (`verhalten_stil`): `verhalten` ist selbst ein Feld.
  if (bekannt.includes(pfad))
    return t(`assistenten.pfad.${pfad.replace(/\./g, "_")}`);
  // Ausspracheregeln: `aussprache[1].wort` heißt „Aussprache, Regel 2, Wort“ (gezählt ab 1, wie die Zeilen im Editor).
  const regel = AUSSPRACHE_PFAD.exec(pfad);
  if (regel) {
    const [, n, feld] = regel;
    const teile = [t("assistenten.pfad.aussprache")];
    if (n !== undefined)
      teile.push(t("assistenten.pfad.aussprache_regel", { n: Number(n) + 1 }));
    if (feld) teile.push(t(`assistenten.pfad.aussprache_${feld}`));
    return teile.join(", ");
  }
  const feld = feldVon(pfad);
  if (feld === "wissen" || feld === "verhalten") {
    const rest = pfad.slice(feld.length);
    return `${t(`assistenten.pfad.${feld}`)}${rest}`;
  }
  return pfad;
}

/** Eine Zeile der Fehlerliste: „Name: hat eine unzulässige Länge“. */
export function fehlerZeile(t: TFunction, f: ProfilFehler): string {
  return `${pfadText(t, f.pfad)}: ${t(`assistenten.code.${f.code}`)}`;
}

export function fehlerZeilen(t: TFunction, fehler: readonly ProfilFehler[]) {
  return fehler.map((f) => fehlerZeile(t, f));
}
