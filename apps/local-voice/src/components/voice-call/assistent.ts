import { commands, type MessungDto } from "@/bindings";
import i18n from "@/i18n";
import {
  sprachwahl,
  type EinstufungErgebnis,
  type Sprache,
} from "./assistentPlan";

/** Sprache der Vorlagen: die der App. */
export function aktuelleSprache(): Sprache {
  return sprachwahl(i18n.language);
}

/**
 * Lokale Einstufung der Äußerung (ADR-0028 E4.1). `einstufung: null`, wenn Ansagen und
 * S0-Vorlagen abgeschaltet sind (`assistent_einstufung_aktiv`). Ein Fehler ist ein eigenes
 * Ergebnis: die Oberfläche zeigt dann die feste Meldung und geht nie ans Modell.
 */
export async function einstufen(
  text: string,
  conversationId: string | null,
): Promise<EinstufungErgebnis> {
  try {
    const r = await commands.assistentEinstufen(
      text,
      i18n.language || "de",
      conversationId,
    );
    return r.status === "ok"
      ? { art: "ok", einstufung: r.data }
      : { art: "fehler" };
  } catch {
    return { art: "fehler" };
  }
}

/** Letzte vom Tor freigegebene Ausgabe des Gesprächs (Rust hält sie); `null` ohne eine. */
export async function letzteAusgabe(
  conversationId: string | null,
): Promise<string | null> {
  try {
    return (await commands.assistentLetzteAusgabe(conversationId)) || null;
  } catch {
    return null;
  }
}

/** Vergisst die letzte Ausgabe (nach einer Antwort, die nicht durchs Tor lief, oder bei neuem Gespräch). */
export async function vergissAusgabe(
  conversationId: string | null,
): Promise<void> {
  try {
    await commands.assistentAusgabeVergessen(conversationId);
  } catch {
    // Ohne Folgen: schlimmstenfalls wird eine ältere Ausgabe wiederholt, nie eine ungeprüfte.
  }
}

/** Messpunkte (E9) ohne Inhalte melden; ein Fehler bleibt ohne Folgen. */
export function meldeMessung(messung: MessungDto): void {
  void commands.assistentMessungMelden(messung).catch(() => undefined);
}
