import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { auspacken, fehlerText, laufText } from "@/lib/projekte";
import { meldeProjekte } from "@/lib/projekteBus";

export interface StartErgebnis {
  /** `fertig`, `freigabe` (der Dialog des Wächters übernimmt), `fehler`. */
  status: "fertig" | "freigabe" | "fehler";
  /** Kennung des Dokuments (`fertig`) oder des Halts (`freigabe`): beide tragen dieselbe Kennung. */
  rechercheId?: string;
  /** Verständlicher Text bei `fehler` (und der Hinweis bei `freigabe`). */
  message?: string;
}

/**
 * Startet eine Recherche (Modus Schnell) in einem Suite-Projekt und meldet das Ergebnis über den
 * Bus: ein Halt an den Freigabe-Wächter, ein fertiges Dokument an die Projektseite. `besprechung`
 * ist die Besprechung, aus der gefragt wird; das Backend prüft, dass das Projekt zu ihr gehört.
 * Mit `mitKontext` nimmt das Backend den Kontext des Projekts hinzu (lva#365 P4c-3): Das Versandtor
 * liest ihn frisch, und der Lauf hält vor der Suche immer an. Ob das erlaubt ist, entscheidet das
 * Backend (Tor), nie die Oberfläche.
 */
export function useRechercheStart() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const start = useCallback(
    async (
      frage: string,
      suiteProjekt: string,
      besprechung: string | null = null,
      mitKontext = false,
    ): Promise<StartErgebnis> => {
      setBusy(true);
      try {
        const a = auspacken(
          await commands.rechercheStarten(
            frage,
            suiteProjekt,
            besprechung,
            mitKontext ? true : null,
          ),
        );
        if (a.status === "fertig" && a.rechercheId) {
          meldeProjekte("recherche", a.rechercheId);
          meldeProjekte("projekte");
          return { status: "fertig", rechercheId: a.rechercheId };
        }
        if (a.status === "freigabe" && a.rechercheId) {
          meldeProjekte("freigabe");
          return {
            status: "freigabe",
            rechercheId: a.rechercheId,
            message: t("projekte.freigabe.wartet"),
          };
        }
        return { status: "fehler", message: laufText(a, t) };
      } catch (e) {
        return { status: "fehler", message: fehlerText(e, t) };
      } finally {
        setBusy(false);
      }
    },
    [t],
  );

  return { start, busy };
}
