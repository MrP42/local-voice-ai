import "./styles";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type SuiteProjekt } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { useProjekteMeldung } from "@/lib/projekteBus";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import { RechercheKarte } from "./RechercheKarte";
import { useRechercheStart } from "./useRechercheStart";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Vorbelegung der Frage (Text der Eingabezeile). */
  frage: string;
}

/**
 * Recherche aus dem Sprachgespräch (Modus Schnell, ADR-0026 PR-7): Frage und Suite-Projekt wählen,
 * recherchieren, Ergebnis lesen und über die vorhandene Sprachausgabe vorlesen. Das Projekt muss das
 * Web freigeben; ab PERSONAL hält der Lauf an und der Freigabe-Wächter fragt nach.
 */
export const SprachRecherche: React.FC<Props> = ({
  open,
  onOpenChange,
  frage: vorgabe,
}) => {
  const { t } = useTranslation();
  const { start, busy } = useRechercheStart();
  const [projekte, setProjekte] = useState<SuiteProjekt[]>([]);
  const [projekt, setProjekt] = useState<string | null>(null);
  const [frage, setFrage] = useState("");
  const [meldung, setMeldung] = useState<string | null>(null);
  const [ergebnis, setErgebnis] = useState<string | null>(null);
  const [erwartet, setErwartet] = useState(false);

  useEffect(() => {
    if (!open) return;
    setFrage(vorgabe);
    setMeldung(null);
    setErgebnis(null);
    commands
      .projekteListe()
      .then((r) => {
        const liste = auspacken(r).filter(
          (p) => !p.archived && p.webResearchEnabled,
        );
        setProjekte(liste);
        setProjekt((cur) => cur ?? liste[0]?.id ?? null);
      })
      .catch((e) => setMeldung(fehlerText(e)));
    // Nur beim Oeffnen vorbelegen.
  }, [open]);

  useProjekteMeldung("recherche", (id) => {
    if (id && erwartet) {
      setErgebnis(id);
      setErwartet(false);
      setMeldung(null);
    }
  });

  const los = async () => {
    if (!projekt || frage.trim() === "") return;
    setMeldung(null);
    setErgebnis(null);
    setErwartet(true);
    const r = await start(frage.trim(), projekt);
    if (r.status === "fertig" && r.rechercheId) {
      setErgebnis(r.rechercheId);
      setErwartet(false);
    } else if (r.status === "freigabe") {
      setMeldung(t("projekte.chat.wartet"));
    } else {
      setErwartet(false);
      setMeldung(r.message ?? t("projekte.fehler.unbekannt"));
    }
  };

  const name = projekte.find((p) => p.id === projekt)?.title;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("projekte.sprache.titel")}
      closeLabel={t("projekte.seite.schliessen")}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("projekte.seite.schliessen")}
          </Button>
          <Button
            variant="primary"
            onClick={() => void los()}
            disabled={busy || !projekt || frage.trim() === ""}
            data-testid="sprache-recherche-los"
          >
            {t("projekte.sprache.los")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="sprache-recherche">
        {projekte.length === 0 ? (
          <p className="text-sm text-text/70">
            {t("projekte.sprache.keinProjekt")}
          </p>
        ) : (
          <>
            <div>
              <label className="mb-1 block text-xs text-text/60">
                {t("projekte.sprache.projekt")}
              </label>
              <Select
                value={projekt}
                options={projekte.map((p) => ({ value: p.id, label: p.title }))}
                ariaLabel={t("projekte.sprache.projekt")}
                onChange={setProjekt}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs text-text/60">
                {t("projekte.sprache.frage")}
              </label>
              <Input
                value={frage}
                aria-label={t("projekte.sprache.frage")}
                onChange={(e) => setFrage(e.target.value)}
              />
            </div>
          </>
        )}
        {meldung && (
          <p role="status" className="text-xs text-text/70">
            {meldung}
          </p>
        )}
        {ergebnis && <RechercheKarte id={ergebnis} projektName={name} />}
      </div>
    </Dialog>
  );
};
