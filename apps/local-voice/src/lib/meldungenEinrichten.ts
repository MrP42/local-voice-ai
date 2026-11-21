import { toast } from "sonner";
import i18n from "@/i18n";
import { openSettingsTab } from "@/lib/openSettingsTab";
import {
  buendeln,
  istSpeicherschutzMeldung,
  type MeldungsAktion,
  type MeldungsFunktion,
} from "./meldungenBuendeln";

/** Anker des Abschnitts „Speicherschutz“ unter Einstellungen › App. */
export const SPEICHERSCHUTZ_ANKER = "speicherschutz";

/** Knopf „Grenze ändern“: springt zum Abschnitt Speicherschutz. */
export const grenzeAendern = (): MeldungsAktion => ({
  label: i18n.t("memoryGuard.changeLimit"),
  onClick: () => openSettingsTab("app", SPEICHERSCHUTZ_ANKER),
});

let eingerichtet = false;

/**
 * Einmal beim Start: `toast.error` und `toast.warning` bündeln gleiche
 * Meldungen (lva#654) und hängen an Speicherschutz-Meldungen den Knopf
 * „Grenze ändern“. Gilt für alle Aufrufer, ohne sie einzeln anzufassen.
 */
export function meldungenEinrichten(): void {
  if (eingerichtet) return;
  eingerichtet = true;
  const aktionFuer = (text: string) =>
    istSpeicherschutzMeldung(text) ? grenzeAendern() : undefined;
  const fehler = buendeln(toast.error as unknown as MeldungsFunktion, {
    praefix: "lv-fehler",
    aktionFuer,
  });
  const warnung = buendeln(toast.warning as unknown as MeldungsFunktion, {
    praefix: "lv-warnung",
    aktionFuer,
  });
  toast.error = fehler as unknown as typeof toast.error;
  toast.warning = warnung as unknown as typeof toast.warning;
  // Ein per Kennung geschlossener Eintrag zählt danach nicht weiter.
  const schliessen = toast.dismiss;
  toast.dismiss = ((id?: string | number) => {
    fehler.vergessen(id);
    warnung.vergessen(id);
    return schliessen(id);
  }) as typeof toast.dismiss;
}
