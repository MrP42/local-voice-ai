import React, { useState } from "react";
import { createPortal } from "react-dom";
import { usePageHeaderSlot } from "../shell/PageHeaderSlot";
import { useTranslation } from "react-i18next";
import { HelpCircle, X } from "lucide-react";
import { HelpPanel } from "../help/HelpPanel";
import { IconButton } from "./IconButton";
import {
  ToggleButton,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";

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
  const slot = usePageHeaderSlot();
  const head = (
    <header className="page-shell__head flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0">
        <h1 id="page-title" className="page-shell__title text-text">
          {title}
        </h1>
      </div>
      {(actions || help) && (
        <div className="page-shell__actions flex items-center gap-1 shrink-0">
          {help && (
            <FluentTooltip content={t("help.open")} relationship="label">
              <ToggleButton
                appearance="subtle"
                checked={helpOpen}
                onClick={() => setHelpOpen((open) => !open)}
                aria-pressed={helpOpen}
                icon={<HelpCircle width={20} height={20} aria-hidden="true" />}
              />
            </FluentTooltip>
          )}
          {actions}
        </div>
      )}
    </header>
  );
  return (
    <section
      className={`page-shell w-full ${fill ? "page-shell--fill flex flex-col gap-4" : "space-y-4"}`}
      aria-labelledby="page-title"
    >
      {slot ? createPortal(head, slot) : head}
      {description && (
        <p className="page-shell__description text-sm text-text-muted">
          {description}
        </p>
      )}
      {help && helpOpen ? (
        <div
          className={`flex gap-4 ${fill ? "flex-1 min-h-0 items-stretch" : "items-start"}`}
        >
          <div className={`flex-1 min-w-0 ${fill ? "min-h-0" : "space-y-4"}`}>
            {children}
          </div>
          <aside
            className={`w-72 shrink-0 space-y-2 border-s border-mid-gray/20 ps-4 ${
              fill ? "min-h-0 overflow-y-auto" : ""
            }`}
            aria-label={t("help.title")}
          >
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-text/50">
                {t("help.title")}
              </span>
              <IconButton
                onClick={() => setHelpOpen(false)}
                label={t("help.close")}
                icon={<X width={14} height={14} />}
              />
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
