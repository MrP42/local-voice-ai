import { commands, type EinstufungDto, type MessungDto } from "@/bindings";
import i18n from "@/i18n";

/**
 * Lokale Einstufung der Äußerung (ADR-0028 E4.1). `null`, wenn die Einstufung abgeschaltet ist
 * (`assistent_einstufung_aktiv`) oder nicht antwortet: dann gilt der bisherige Ablauf.
 */
export async function einstufen(text: string): Promise<EinstufungDto | null> {
  try {
    const r = await commands.assistentEinstufen(text, i18n.language || "de");
    return r.status === "ok" ? r.data : null;
  } catch {
    return null;
  }
}

/** Messpunkte (E9) ohne Inhalte melden; ein Fehler bleibt ohne Folgen. */
export function meldeMessung(messung: MessungDto): void {
  void commands.assistentMessungMelden(messung).catch(() => undefined);
}
