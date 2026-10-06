import React from "react";
import { useTranslation } from "react-i18next";

/**
 * Oben in jedem Bereich: welches Modell gerade arbeitet. Die eine Frage, die
 * man auf der Modellseite fast immer zuerst hat.
 */
export const ActiveBar: React.FC<{
  label: string;
  name: string | null;
  chips?: React.ReactNode;
  /** Rechts: Laufzeit, Zusatzinfo. */
  aside?: React.ReactNode;
}> = ({ label, name, chips, aside }) => {
  const { t } = useTranslation();
  return (
    <div
      className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-logo-primary/30 bg-logo-primary/5 px-3 py-2"
      data-active-bar
    >
      <span className="text-xs font-semibold uppercase tracking-wide text-text/60">
        {label}
      </span>
      <span className="text-sm font-semibold text-text" data-active-name>
        {name ?? t("settings.models.active.none")}
      </span>
      {chips}
      {aside && (
        <span className="ms-auto text-xs text-text/50">{aside}</span>
      )}
    </div>
  );
};

export default ActiveBar;
