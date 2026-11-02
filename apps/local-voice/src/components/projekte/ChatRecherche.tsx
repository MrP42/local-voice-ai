import "./styles";
import React, { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe } from "lucide-react";
import { commands, type SuiteProjekt } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { useProjekteMeldung } from "@/lib/projekteBus";
import { Button } from "../ui/Button";
import { RechercheKarte } from "./RechercheKarte";
import { useRechercheStart } from "./useRechercheStart";

/**
 * Werkzeug "recherche" der Chat-Spalte (Modus Schnell, ADR-0026 PR-7). Der Chat der Besprechung
 * ist ein fester Ablauf ohne Werkzeugwahl des Modells; die Recherche ist deshalb eine ausdrückliche
 * Aktion der Person: die eingegebene Frage geht an das Suite-Projekt, mit dem der Ordner der
 * Besprechung verknüpft ist. Ab PERSONAL (Besprechungen) hält der Lauf an; der Freigabe-Wächter
 * zeigt dann den Dialog mit der Frage und den Suchanfragen. Das Ergebnis liegt im Projektordner.
 *
 * Gibt zurück: `knopf` (neben "Senden"), `unten` (Wahl des Projekts, Ergebnis) und `zustand`.
 */
export function useChatRecherche(
  meetingId: string | null,
  frage: string,
  onGestartet: () => void,
  onHinweis: (text: string | null) => void,
) {
  const { t } = useTranslation();
  const { start, busy } = useRechercheStart();
  const [wahl, setWahl] = useState<{
    frage: string;
    projekte: SuiteProjekt[];
  } | null>(null);
  const [karte, setKarte] = useState<{ id: string; projekt: string } | null>(
    null,
  );
  const erwartet = useRef<string | null>(null);

  useProjekteMeldung("recherche", (id) => {
    if (id && erwartet.current !== null) {
      setKarte({ id, projekt: erwartet.current });
      erwartet.current = null;
      onHinweis(null);
    }
  });

  const laufen = useCallback(
    async (text: string, projekt: SuiteProjekt) => {
      setWahl(null);
      erwartet.current = projekt.title;
      onGestartet();
      const r = await start(text, projekt.id);
      if (r.status === "fertig" && r.rechercheId) {
        // Das Ergebnis kommt auch über den Bus; hier nur sichern, falls die Meldung schon durch ist.
        setKarte({ id: r.rechercheId, projekt: projekt.title });
        erwartet.current = null;
        onHinweis(null);
      } else if (r.status === "freigabe") {
        onHinweis(t("projekte.chat.wartet"));
      } else {
        erwartet.current = null;
        onHinweis(r.message ?? t("projekte.fehler.unbekannt"));
      }
    },
    [start, onGestartet, onHinweis, t],
  );

  const los = useCallback(async () => {
    const text = frage.trim();
    if (!meetingId || text === "") return;
    onHinweis(null);
    try {
      const kandidaten = auspacken(
        await commands.projekteDerBesprechung(meetingId),
      );
      if (kandidaten.length === 0) {
        onHinweis(t("projekte.chat.keinProjekt"));
      } else if (kandidaten.length === 1) {
        await laufen(text, kandidaten[0]);
      } else {
        setWahl({ frage: text, projekte: kandidaten });
      }
    } catch (e) {
      onHinweis(fehlerText(e));
    }
  }, [frage, meetingId, laufen, onHinweis, t]);

  const knopf = meetingId ? (
    <Button
      size="sm"
      variant="secondary"
      onClick={() => void los()}
      disabled={busy || frage.trim() === ""}
      title={t("projekte.chat.hinweis")}
      data-testid="chat-recherche"
      icon={<Globe width={12} height={12} aria-hidden="true" />}
    >
      {t("projekte.chat.recherche")}
    </Button>
  ) : null;

  const unten = (
    <>
      {wahl && (
        <div
          role="group"
          aria-label={t("projekte.chat.projektWaehlen")}
          data-testid="chat-recherche-wahl"
          className="flex flex-wrap items-center gap-1 text-xs"
        >
          <span className="text-text/60">
            {t("projekte.chat.projektWaehlen")}
          </span>
          {wahl.projekte.map((p) => (
            <Button
              key={p.id}
              size="sm"
              variant="secondary"
              onClick={() => void laufen(wahl.frage, p)}
            >
              {p.title}
            </Button>
          ))}
        </div>
      )}
      {karte && (
        <RechercheKarte
          id={karte.id}
          projektName={karte.projekt}
          onClose={() => setKarte(null)}
        />
      )}
    </>
  );

  return { knopf, unten, busy };
}
