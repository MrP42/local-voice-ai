import "./styles";
import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type FreigabeAktion, type FreigabeInfo } from "@/bindings";
import {
  auspacken,
  fehlerText,
  freigabeAnfragen,
  freigabeKontext,
} from "@/lib/projekte";
import { meldeProjekte, useProjekteMeldung } from "@/lib/projekteBus";
import { RechercheFreigabe } from "../../vendor/wai-ui/projekt";
import { useFreigabeLabels } from "./labels";

/** Wie oft nach abgelaufenen Halten gesehen wird, solange einer offen ist. */
const PRUEFEN_MS = 20_000;

/**
 * Wächter der Recherche-Freigabe (ADR-0026): zeigt den Dialog, sobald eine Recherche ab der Klasse
 * PERSONAL angehalten wurde. Gesendet wird nur, was die Person ankreuzt; "Nur im Projekt suchen" und
 * "Abbrechen" senden nichts. Der Dialog nennt die Frage der Recherche und die Datenklasse, so wie sie
 * das Backend festhält. Einmal pro App einbinden (Aufnahmen-Seite); Chat-Spalte, Sprachgespräch und
 * Projektseite melden über `meldeProjekte("freigabe")`.
 */
export const FreigabeWaechter: React.FC = () => {
  const { t } = useTranslation();
  const labels = useFreigabeLabels();
  const [offen, setOffen] = useState<FreigabeInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [fehler, setFehler] = useState<string | undefined>();

  const laden = useCallback(async () => {
    try {
      setOffen(auspacken(await commands.rechercheFreigaben()));
    } catch {
      setOffen([]);
    }
  }, []);

  useEffect(() => {
    void laden();
  }, [laden]);
  useProjekteMeldung("freigabe", () => void laden());
  useEffect(() => {
    if (offen.length === 0) return;
    const id = window.setInterval(() => void laden(), PRUEFEN_MS);
    return () => window.clearInterval(id);
  }, [offen.length, laden]);

  const aktuell = offen[0];

  const entscheiden = async (
    aktion: FreigabeAktion,
    ids: readonly string[],
  ) => {
    if (!aktuell || busy) return;
    setBusy(true);
    setFehler(undefined);
    try {
      const a = auspacken(
        await commands.rechercheFreigabeEntscheiden(
          aktuell.id,
          [...ids],
          aktion,
        ),
      );
      if (a.status === "fehler") {
        setFehler(a.message ?? t("projekte.fehler.unbekannt"));
      } else if (a.status === "fertig" && a.rechercheId) {
        toast.success(t("projekte.freigabe.fertig"));
        meldeProjekte("recherche", a.rechercheId);
        meldeProjekte("projekte");
      } else if (a.status === "abgelehnt") {
        toast.info(t("projekte.freigabe.nichtsGesendet"));
      }
    } catch (e) {
      setFehler(fehlerText(e));
    } finally {
      setBusy(false);
      await laden();
    }
  };

  return (
    <RechercheFreigabe
      key={aktuell?.id ?? "keine"}
      isOpen={aktuell !== undefined}
      onOpenChange={() => undefined}
      contextLabel={aktuell ? freigabeKontext(aktuell) : ""}
      queries={aktuell ? freigabeAnfragen(aktuell) : []}
      busy={busy}
      error={fehler}
      labels={labels}
      onSend={(ids) => void entscheiden("senden", ids)}
      onProjectOnly={() => void entscheiden("nur_projekt", [])}
      onCancel={() => {
        if (!busy) void entscheiden("abbrechen", []);
      }}
    />
  );
};
