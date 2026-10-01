import React from "react";
import { useTranslation } from "react-i18next";
import { FileText } from "lucide-react";
import type { ProjectMinutesSummary } from "@/bindings";
import type { LiveProgress } from "@/lib/meetingJobs";
import { formatMeetingDate } from "@/lib/meetingDate";
import { kindTitleKey } from "@/lib/projectMinutes";
import { JobBar } from "../JobProgress";

interface ProjectMinutesRowsProps {
  items: ProjectMinutesSummary[];
  /** Fortschritt des laufenden Laufs dieses Projekts (Verzeichnis der Verarbeitungen). */
  progress: LiveProgress | undefined;
  /** Das gerade geoeffnete Dokument, falls es eines dieses Projekts ist. */
  activeId: string | null;
  /** Die Ansicht des laufenden Laufs ist offen. */
  runActive: boolean;
  onOpen: (id: string) => void;
  onOpenRun: () => void;
}

/**
 * Die Projekt-Protokolle eines Projekts, als Eintraege ueber den Aufnahmen in der
 * Liste: ein laufender Lauf mit Fortschritt zuerst, dann die fertigen, das juengste
 * vorn. Ein Klick oeffnet das Dokument in der Arbeitsflaeche.
 */
export const ProjectMinutesRows: React.FC<ProjectMinutesRowsProps> = ({
  items,
  progress,
  activeId,
  runActive,
  onOpen,
  onOpenRun,
}) => {
  const { t, i18n } = useTranslation();
  if (!progress && items.length === 0) return null;

  const rowClass = (active: boolean) =>
    `my-px ms-5 cursor-pointer select-none rounded-lg border px-2 py-1.5 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 ${
      active
        ? "border-transparent bg-logo-primary/15"
        : "border-transparent hover:bg-mid-gray/10"
    }`;
  const onKey = (action: () => void) => (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      action();
    }
  };

  return (
    <div
      role="group"
      aria-label={t("meetings.projectMinutes.list.heading")}
      data-testid="pm-list"
      className="pb-1"
    >
      <p className="ms-5 px-2 pt-1 text-[11px] font-semibold uppercase tracking-wide text-text/60">
        {t("meetings.projectMinutes.list.heading")}
      </p>
      {progress && (
        <div
          role="button"
          tabIndex={0}
          data-testid="pm-row-running"
          aria-current={runActive ? "true" : undefined}
          className={rowClass(runActive)}
          onClick={onOpenRun}
          onKeyDown={onKey(onOpenRun)}
        >
          <div className="flex items-center gap-1.5">
            <FileText width={14} height={14} aria-hidden="true" />
            <p className="min-w-0 flex-1 truncate text-sm font-medium">
              {t("meetings.projectMinutes.list.running")}
            </p>
          </div>
          {/* Die Leiste haelt Klicks von der Zeile fern (in der Besprechungsliste
              oeffnen sie sonst die Besprechung); hier soll ein Klick darauf den
              Lauf oeffnen, also schon in der Erfassungsphase abfangen. */}
          <div onClickCapture={onOpenRun}>
            <JobBar progress={progress} className="mt-1 w-full" />
          </div>
        </div>
      )}
      {items.map((item) => {
        const active = activeId === item.id;
        const date = formatMeetingDate(
          new Date(item.created_at * 1000),
          i18n.language,
          "compact",
        );
        return (
          <div
            key={item.id}
            role="button"
            tabIndex={0}
            data-testid="pm-row"
            data-pm-id={item.id}
            data-kind={item.kind}
            aria-current={active ? "true" : undefined}
            className={rowClass(active)}
            onClick={() => onOpen(item.id)}
            onKeyDown={onKey(() => onOpen(item.id))}
          >
            <div className="flex items-center gap-1.5">
              <FileText width={14} height={14} aria-hidden="true" />
              <p className="min-w-0 flex-1 truncate text-sm font-medium">
                {t(kindTitleKey(item.kind))}
              </p>
              {item.incomplete && (
                <span
                  className="shrink-0 rounded-full bg-amber-500/20 px-2 text-[11px] font-medium text-amber-700 dark:text-amber-400"
                  data-testid="pm-row-incomplete"
                >
                  {t("meetings.projectMinutes.list.incomplete")}
                </span>
              )}
            </div>
            <p className="truncate text-xs text-text/60">
              {t("meetings.projectMinutes.list.meta", {
                date,
                count: item.recordings,
              })}
            </p>
          </div>
        );
      })}
    </div>
  );
};
