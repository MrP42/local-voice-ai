import React from "react";
import { useTranslation } from "react-i18next";
import { FolderOpen } from "lucide-react";
import { oeffneProjekt } from "@/lib/projekteBus";
import { anzahlVon, useProjekteStand } from "./useProjekte";
import "./projektbereich.css";

/**
 * Gruppe „Projekte“ in der Leiste von Vorlesen und Sprachgespräch (lva#365 P4b): die Projekte mit
 * der Zahl ihrer Einträge (dieselbe Quelle wie im Bereich Projekte). Ein Klick öffnet die
 * Projektseite im Bereich „Projekte“. Ohne Projekte erscheint nichts.
 */
export const ProjekteGruppe: React.FC<{
  /** Vor dem Wechsel (z. B. Schublade der schmalen Ansicht schließen). */
  onGewaehlt?: () => void;
}> = ({ onGewaehlt }) => {
  const { t } = useTranslation();
  const stand = useProjekteStand();
  if (stand.projekte.length === 0) return null;
  return (
    <nav
      aria-label={t("projektBereich.gruppeLeiste")}
      data-testid="projekte-gruppe"
      className="pb-gruppe"
    >
      <p className="pb-grp">{t("projektBereich.gruppeLeiste")}</p>
      {stand.projekte.map((p) => (
        <button
          key={p.id}
          type="button"
          className="pb-row"
          data-testid="projekte-gruppe-zeile"
          data-projekt={p.id}
          title={p.title}
          onClick={() => {
            onGewaehlt?.();
            oeffneProjekt(p.id);
          }}
        >
          <FolderOpen width={18} height={18} aria-hidden="true" />
          <span className="pb-row__nm">{p.title}</span>
          <span className="pb-row__ct">{anzahlVon(stand, p.id)}</span>
        </button>
      ))}
    </nav>
  );
};
