import { useSettings } from "../../hooks/useSettings";
import { alsKlasse } from "@/lib/projekte";

const ORDNUNG = ["PUBLIC", "INTERNAL", "PERSONAL", "SPECIAL", "RESTRICTED"];

/**
 * Klassenregel für das Vorlesen einer Recherche: ab PERSONAL darf der Text nur an eine lokale
 * Sprachausgabe gehen. Liest ein Anbieter vor (`tts_engine` = `provider:<id>`), dessen Zugriff
 * außerhalb von Loopback erlaubt ist (`tts_providers.allow_remote`), ist der Knopf gesperrt.
 */
export function useVorlesenGesperrt(
  klasse: string | null | undefined,
): boolean {
  const { getSetting } = useSettings();
  const motor = getSetting("tts_engine") ?? "";
  if (ORDNUNG.indexOf(alsKlasse(klasse)) < ORDNUNG.indexOf("PERSONAL")) {
    return false;
  }
  if (!motor.startsWith("provider:")) return false;
  const id = motor.slice("provider:".length);
  return (getSetting("tts_providers")?.allow_remote ?? []).includes(id);
}
