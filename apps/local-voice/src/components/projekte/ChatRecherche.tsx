import "./styles";
import React, { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe } from "lucide-react";
import { commands, type SuiteProjekt } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { useErwartung, type HaltEnde } from "@/lib/projekteBus";
import { Button } from "../ui/Button";
import { RechercheKarte } from "./RechercheKarte";
import { useRechercheStart } from "./useRechercheStart";

/**
 * Werkzeug "recherche" der Chat-Spalte (Modus Schnell, ADR-0026 PR-7). Der Chat der Besprechung
 * ist ein fester Ablauf ohne Werkzeugwahl des Modells; die Recherche ist deshalb eine ausdrückliche
 * Aktion der Person: die eingegebene Frage geht an das Suite-Projekt, mit dem der Ordner der
 * Besprechung verknüpft ist (das Backend prüft die Zugehörigkeit). Ab PERSONAL hält der Lauf an;
 * der Freigabe-Wächter zeigt dann den Dialog mit der Frage und den Suchanfragen. Das Ergebnis wird
 * über die Kennung des Halts bzw. der Recherche zugeordnet und liegt im Projektordner.
 *
 * Gibt zurück: `knopf` (neben "Senden"), `unten` (Wahl des Projekts, Ergebnis) und `busy`.
 * `onGestartet` (Eingabefeld leeren) kommt erst, wenn der Start angenommen wurde.
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
  const [projektVon, setProjektVon] = useState<Record<string, string>>({});

  const erwartung = useErwartung(
    (id) => {
      setKarte({ id, projekt: projektVon[id] ?? "" });
      onHinweis(null);
    },
    (_id, wie: HaltEnde) => {
      setKarte(null);
      onHinweis(
        wie === "abgelehnt"
          ? t("projekte.freigabe.nichtsGesendet")
          : wie === "abgelaufen"
            ? t("projekte.freigabe.abgelaufen")
            : t("projekte.fehler.unbekannt"),
      );
    },
  );

  const laufen = useCallback(
    async (text: string, projekt: SuiteProjekt) => {
      setWahl(null);
      setKarte(null);
      erwartung.leeren();
      const r = await start(text, projekt.id, meetingId);
      if (r.status === "fehler") {
        onHinweis(r.message ?? t("projekte.fehler.unbekannt"));
        return;
      }
      onGestartet();
      if (!r.rechercheId) return;
      if (r.status === "fertig") {
        setKarte({ id: r.rechercheId, projekt: projekt.title });
        onHinweis(null);
        return;
      }
      setProjektVon((m) => ({
        ...m,
        [r.rechercheId as string]: projekt.title,
      }));
      erwartung.erwarte(r.rechercheId);
      onHinweis(t("projekte.chat.wartet"));
    },
    // erwartung ist je Render neu, aber nur ueber stabile Refs wirksam
    [start, meetingId, onGestartet, onHinweis, t],
  );

  const los = useCallback(async () => {
    const text = frage.trim();
    if (!meetingId || text === "") return;
    onHinweis(null);
    try {
      const kandidaten = auspacken(
        await commands.projekteDerBesprechung(meetingId),
      ).filter((p) => p.webResearchEnabled);
      if (kandidaten.length === 0) {
        onHinweis(t("projekte.chat.keinProjekt"));
      } else if (kandidaten.length === 1) {
        await laufen(text, kandidaten[0]);
      } else {
        setWahl({ frage: text, projekte: kandidaten });
      }
    } catch (e) {
      onHinweis(fehlerText(e, t));
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
          projektName={karte.projekt || undefined}
          onClose={() => setKarte(null)}
        />
      )}
    </>
  );

  return { knopf, unten, busy };
}
