import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { meldeProjekte } from "@/lib/projekteBus";

export interface StartErgebnis {
  /** `fertig`, `freigabe` (der Dialog des Wächters übernimmt), `fehler`. */
  status: "fertig" | "freigabe" | "fehler";
  rechercheId?: string;
  /** Verständlicher Text bei `fehler` (und der Hinweis bei `freigabe`). */
  message?: string;
}

/**
 * Startet eine Recherche (Modus Schnell) in einem Suite-Projekt und meldet das Ergebnis über den
 * Bus: ein Halt an den Freigabe-Wächter, ein fertiges Dokument an die Projektseite.
 */
export function useRechercheStart() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const start = useCallback(
    async (frage: string, suiteProjekt: string): Promise<StartErgebnis> => {
      setBusy(true);
      try {
        const a = auspacken(
          await commands.rechercheStarten(frage, suiteProjekt),
        );
        if (a.status === "fertig" && a.rechercheId) {
          meldeProjekte("recherche", a.rechercheId);
          meldeProjekte("projekte");
          return { status: "fertig", rechercheId: a.rechercheId };
        }
        if (a.status === "freigabe") {
          meldeProjekte("freigabe");
          return {
            status: "freigabe",
            message: a.message ?? t("projekte.freigabe.wartet"),
          };
        }
        return {
          status: "fehler",
          message: a.message ?? t("projekte.fehler.unbekannt"),
        };
      } catch (e) {
        return { status: "fehler", message: fehlerText(e) };
      } finally {
        setBusy(false);
      }
    },
    [t],
  );

  return { start, busy };
}
