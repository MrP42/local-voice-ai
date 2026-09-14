import React from "react";

interface PageShellProps {
  /** Seitentitel — eine Zeile, dieselbe Groesse auf jeder Seite. */
  title: string;
  /** Ein Satz darunter, was man hier tut. */
  description?: string;
  /** Rechts im Kopf: Schalter, Statussymbole, ein Knopf. Kein Menue. */
  actions?: React.ReactNode;
  /** Nur die Hauptspalte deckeln — Arbeitsflaechen (Vorlesen, Aufnahmen)
      brauchen die volle Breite, Listen und Einstellungen nicht. */
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
  children,
}) => (
  <section className="page-shell w-full space-y-4" aria-labelledby="page-title">
    <header className="page-shell__head flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0">
        <h1 id="page-title" className="text-xl font-semibold text-text">
          {title}
        </h1>
        {description && (
          <p className="text-sm text-text/60 mt-1">{description}</p>
        )}
      </div>
      {actions && (
        <div className="page-shell__actions flex items-center gap-1 shrink-0">
          {actions}
        </div>
      )}
    </header>
    {children}
  </section>
);
