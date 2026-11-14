/**
 * Welcher Assistent im offenen Gespräch gilt, für die Anzeige (lva#556). Die Auswahl (Gesprächsspeicher der Assistenten) und
 * der Assistent, mit dem das Gespräch läuft, fallen auseinander, wenn die Auswahl erst nach dem Beginn des Gesprächs eintrifft
 * (spät geladen, Datei geändert): Rolle und Begrenzung gehören zum Beginn eines Gesprächs, die neue Auswahl gilt ab dem
 * nächsten (`engine.setAssistent`). Die Anzeige (Begrüßung, Bereichs-Chip) richtet sich nach dem, was tatsächlich gilt.
 */
import { useMemo } from "react";
import { aktuelleWirkung, useAssistentStore } from "@/stores/assistentStore";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { geltender, type AssistentWirkung } from "./wirkung";

export interface GeltenderAssistent {
  /** Der Assistent, der im offenen Gespräch wirkt; `null` mit „Alles“ und im Projektgespräch (dort wirkt keiner). */
  wirkung: AssistentWirkung | null;
  /** Die Auswahl ist eine andere und gilt erst ab dem nächsten Gespräch. */
  ausstehend: boolean;
}

export function useGeltenderAssistent(): GeltenderAssistent {
  const assistenten = useAssistentStore((s) => s.assistenten);
  const stand = useVoiceCallStore((s) => s.assistentStand);
  const imProjekt = useVoiceCallStore((s) => s.projectId !== null);
  const gewaehlt = useMemo(
    () => aktuelleWirkung({ assistenten }),
    [assistenten],
  );
  return geltender(imProjekt, stand, gewaehlt);
}
