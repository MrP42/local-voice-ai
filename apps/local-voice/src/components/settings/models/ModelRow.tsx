import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  Loader2,
  MoreHorizontal,
} from "lucide-react";
import { ActionMenu, type ActionMenuItem } from "../../ui/ActionMenu";
import { Button } from "../../ui/Button";

/** Laufender Vorgang einer Zeile: Download mit Prozent, sonst ein Kreisel. */
export type RowProgress =
  | { kind: "download"; percent: number; speed?: number; onCancel?: () => void }
  | { kind: "busy"; label: string };

interface ModelRowProps {
  name: string;
  /** Kleine Kennzeichen hinter dem Namen (Herkunft, Status). */
  badges?: React.ReactNode;
  /** Rechts vor den Aktionen: Groesse, Speicherampel. */
  meta?: React.ReactNode;
  /** Die eine naheliegende Aktion (Verwenden, Laden, Pruefen). */
  primary?: React.ReactNode;
  /** Alles Weitere im Menue hinter "⋯". */
  menu?: ActionMenuItem[];
  progress?: RowProgress | null;
  /** Beschreibung, Merkmale, Hinweise -- auf Klick aufgeklappt. */
  details?: React.ReactNode;
  active?: boolean;
  dimmed?: boolean;
  /** data-Attribute fuer Tests und Styles (z. B. `data-llm-card`). */
  dataAttrs?: Record<string, string | undefined>;
}

/**
 * Ein Modell in einer Zeile: Name, Kennzeichen, Groesse, Aktion. Was man
 * nur selten braucht -- Beschreibung, Merkmale, Pfad, Hinweise -- klappt
 * erst auf Klick auf. Ein laufender Download bleibt immer sichtbar.
 */
export const ModelRow: React.FC<ModelRowProps> = ({
  name,
  badges,
  meta,
  primary,
  menu,
  progress,
  details,
  active = false,
  dimmed = false,
  dataAttrs,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const hasDetails =
    details !== undefined && details !== null && details !== false;
  const visibleMenu = (menu ?? []).filter(Boolean);

  return (
    <div
      className={`border-b border-mid-gray/15 last:border-b-0 ${
        active ? "bg-logo-primary/5" : ""
      }`}
      data-model-row
      data-open={open || undefined}
      {...dataAttrs}
    >
      <div className="flex min-h-11 items-center gap-2 px-3 py-1.5">
        <button
          type="button"
          onClick={() => hasDetails && setOpen(!open)}
          disabled={!hasDetails}
          aria-expanded={hasDetails ? open : undefined}
          aria-label={t("settings.models.row.details", { name })}
          className="flex min-w-0 flex-1 items-center gap-2 rounded text-start focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary disabled:cursor-default"
        >
          <span className="w-4 shrink-0 text-text/40">
            {hasDetails &&
              (open ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              ))}
          </span>
          <span
            className={`truncate text-sm font-medium ${
              dimmed ? "text-text/50" : "text-text"
            }`}
          >
            {name}
          </span>
          {badges && (
            <span className="flex min-w-0 shrink items-center gap-1.5 overflow-hidden">
              {badges}
            </span>
          )}
        </button>
        {meta && (
          <span className="hidden shrink-0 items-center gap-2 text-xs text-text/50 sm:flex">
            {meta}
          </span>
        )}
        <span className="flex shrink-0 items-center gap-1">
          {primary}
          {visibleMenu.length > 0 && (
            <ActionMenu
              trigger={{
                icon: MoreHorizontal,
                label: t("settings.models.row.more", { name }),
                description: t("settings.models.row.more", { name }),
                testId: "row-menu",
                size: "sm",
              }}
              items={visibleMenu}
              menuLabel={t("settings.models.row.more", { name })}
              align="end"
              widthClass="w-56"
            />
          )}
        </span>
      </div>
      {progress && <RowProgressBar progress={progress} />}
      {open && hasDetails && (
        <div className="space-y-2 px-3 pb-3 ps-9 text-sm text-text/70">
          {details}
        </div>
      )}
    </div>
  );
};

const RowProgressBar: React.FC<{ progress: RowProgress }> = ({ progress }) => {
  const { t } = useTranslation();
  if (progress.kind === "busy") {
    return (
      <div className="flex items-center gap-1.5 px-3 pb-2 ps-9 text-xs text-text/50">
        <Loader2 className="h-3 w-3 animate-spin" />
        {progress.label}
      </div>
    );
  }
  return (
    <div className="px-3 pb-2 ps-9" data-row-progress>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-mid-gray/20">
        <div
          className="h-full rounded-full bg-logo-primary transition-all duration-300"
          style={{ width: `${progress.percent}%` }}
        />
      </div>
      <div className="mt-1 flex items-center justify-between text-xs text-text/50">
        <span>
          {t("settings.models.row.downloading", {
            percentage: Math.round(progress.percent),
          })}
          {progress.speed !== undefined && progress.speed > 0 && (
            <span className="ms-2 tabular-nums">
              {t("modelSelector.downloadSpeed", {
                speed: progress.speed.toFixed(1),
              })}
            </span>
          )}
        </span>
        {progress.onCancel && (
          <Button variant="danger-ghost" size="sm" onClick={progress.onCancel}>
            {t("settings.models.row.cancel")}
          </Button>
        )}
      </div>
    </div>
  );
};

/** Kleines Kennzeichen in einer Zeile (Herkunft, Status). */
export const RowChip: React.FC<{
  children: React.ReactNode;
  tone?: "neutral" | "success" | "warning" | "accent";
  title?: string;
  dataAttrs?: Record<string, string | undefined>;
}> = ({ children, tone = "neutral", title, dataAttrs }) => {
  const tones = {
    neutral: "bg-mid-gray/15 text-text/60",
    success: "bg-green-500/15 text-green-600 dark:text-green-400",
    warning: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
    accent: "bg-logo-primary/20 text-text",
  };
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium leading-none ${tones[tone]}`}
      title={title}
      {...dataAttrs}
    >
      {children}
    </span>
  );
};

/** Ampelpunkt: passt / knapp / passt nicht / unbekannt. */
export const FitDot: React.FC<{ verdict: string; title: string }> = ({
  verdict,
  title,
}) => {
  const color =
    verdict === "fits"
      ? "bg-green-500"
      : verdict === "tight"
        ? "bg-yellow-500"
        : verdict === "unlikely"
          ? "bg-red-500"
          : "bg-mid-gray/50";
  return (
    <span
      className={`inline-block h-2 w-2 rounded-full ${color}`}
      title={title}
      aria-label={title}
      data-fit={verdict}
    />
  );
};

export default ModelRow;
