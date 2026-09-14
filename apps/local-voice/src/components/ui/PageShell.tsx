import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { HelpCircle, X } from "lucide-react";
import { HelpPanel } from "../help/HelpPanel";

interface PageShellProps {
  /** Seitentitel — eine Zeile, dieselbe Groesse auf jeder Seite. */
  title: string;
  /** Ein Satz darunter, was man hier tut. */
  description?: string;
  /** Rechts im Kopf: Schalter, Statussymbole, ein Knopf. Kein Menue. */
  actions?: React.ReactNode;
  /** Bereich der Kontexthilfe (`src/content/help/<bereich>.<sprache>.md`).
      Gesetzt: Hilfe-Knopf im Kopf, Hilfe als Spalte rechts neben dem Inhalt.
      Vorlesen bringt seine Hilfe in der eigenen rechten Leiste mit und
      setzt das nicht. */
  help?: string;
  /** Arbeitsflaeche: nimmt die volle Hoehe, der Inhalt scrollt in seinen
      Spalten, der Kopf steht. Listen und Einstellungen lassen das aus und
      scrollen als Ganzes. */
  fill?: boolean;
  children: React.ReactNode;
}

/**
 * Der gemeinsame Rahmen aller Inhaltsseiten: Kopfzeile mit Titel,
 * Beschreibung und Aktionen, darunter der Inhalt. Vorher hatte jede Seite
 * ihren eigenen Kopf (h1 hier, h2 dort, keiner bei den Einstellungen) — das
 * sah nach fuenf Apps aus. Menue- und Fussleiste bleiben unberuehrt.
 */
export const PageShell: React.FC<PageShellProps> = ({
  title,
  description,
  actions,
  help,
  fill = false,
  children,
}) => {
  const { t } = useTranslation();
  const [helpOpen, setHelpOpen] = useState(false);
  return (
    <section
      className={`page-shell w-full ${fill ? "page-shell--fill flex flex-col gap-4" : "space-y-4"}`}
      aria-labelledby="page-title"
    >
      <header className="page-shell__head flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 id="page-title" className="text-xl font-semibold text-text">
            {title}
          </h1>
          {description && (
            <p className="text-sm text-text/60 mt-1">{description}</p>
          )}
        </div>
        {(actions || help) && (
          <div className="page-shell__actions flex items-center gap-1 shrink-0">
            {help && (
              <button
                type="button"
                onClick={() => setHelpOpen((open) => !open)}
                title={t("help.open")}
                aria-label={t("help.open")}
                aria-pressed={helpOpen}
                className={`p-1.5 rounded-md hover:bg-mid-gray/20 transition-colors cursor-pointer ${
                  helpOpen ? "text-text" : "text-text/60 hover:text-text"
                }`}
              >
                <HelpCircle width={20} height={20} aria-hidden="true" />
              </button>
            )}
            {actions}
          </div>
        )}
      </header>
      {help && helpOpen ? (
        <div className={`flex gap-4 items-start ${fill ? "flex-1 min-h-0" : ""}`}>
          <div className="flex-1 min-w-0 space-y-4">{children}</div>
          <aside
            className="w-72 shrink-0 space-y-2 border-s border-mid-gray/20 ps-4"
            aria-label={t("help.title")}
          >
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold uppercase tracking-wide text-text/50">
                {t("help.title")}
              </span>
              <button
                type="button"
                onClick={() => setHelpOpen(false)}
                title={t("help.close")}
                aria-label={t("help.close")}
                className="p-1 rounded-md text-text/50 hover:text-text hover:bg-mid-gray/20 cursor-pointer"
              >
                <X width={14} height={14} />
              </button>
            </div>
            <HelpPanel section={help} />
          </aside>
        </div>
      ) : fill ? (
        <div className="flex-1 min-h-0">{children}</div>
      ) : (
        children
      )}
    </section>
  );
};
