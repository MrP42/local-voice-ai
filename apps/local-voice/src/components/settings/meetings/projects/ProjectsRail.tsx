import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { Folder as FolderIcon, FolderMinus, Folders } from "lucide-react";
import type { Folder } from "@/bindings";
import { TooltipTrigger } from "../../../ui/TooltipTrigger";
import { ALL_PROJECTS, NO_PROJECT, abbreviation } from "./projectModel";

interface RailItemProps {
  id: string;
  label: string;
  hint: string;
  selected: boolean;
  onPick: () => void;
  children: React.ReactNode;
}

const RailItem: React.FC<RailItemProps> = ({
  id,
  label,
  hint,
  selected,
  onPick,
  children,
}) => {
  const tooltipId = `${useId()}-tip`;
  return (
    <TooltipTrigger
      tooltipId={tooltipId}
      content={
        <>
          <div className="text-sm font-semibold">
            <strong>{label}</strong>
          </div>
          <div className="text-xs text-text/70">{hint}</div>
        </>
      }
    >
      <button
        type="button"
        data-testid="rail-project"
        data-project-id={id}
        aria-label={label}
        aria-describedby={tooltipId}
        aria-current={selected ? "true" : undefined}
        onClick={onPick}
        className={`inline-flex h-[30px] w-[30px] cursor-pointer items-center justify-center rounded-lg border text-xs font-semibold transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary ${
          selected
            ? "border-transparent bg-logo-primary/20 text-text"
            : "border-transparent text-text/60 hover:bg-mid-gray/15 hover:text-text"
        }`}
      >
        {children}
      </button>
    </TooltipTrigger>
  );
};

interface ProjectsRailProps {
  folders: Folder[];
  counts: { all: number | null; none: number | null };
  selection: string;
  /** Waehlt das Projekt und oeffnet die Spalte bzw. die Schublade. */
  onPick: (id: string) => void;
}

/** Eingeklappte Projekte-Spalte: je Projekt ein Kuerzel, dazu "Alle" und "Ohne Projekt". */
export const ProjectsRail: React.FC<ProjectsRailProps> = ({
  folders,
  counts,
  selection,
  onPick,
}) => {
  const { t } = useTranslation();
  const countHint = (n: number | null) =>
    n === null ? "" : t("meetings.projects.meetingCount", { count: n });
  return (
    <div
      className="mt-1 flex min-h-0 flex-col items-center gap-1 overflow-y-auto"
      data-testid="projects-rail"
    >
      <RailItem
        id={ALL_PROJECTS}
        label={t("meetings.projects.all")}
        hint={countHint(counts.all)}
        selected={selection === ALL_PROJECTS}
        onPick={() => onPick(ALL_PROJECTS)}
      >
        <Folders width={16} height={16} aria-hidden="true" />
      </RailItem>
      {folders.map((folder) => (
        <RailItem
          key={folder.id}
          id={folder.id}
          label={folder.name}
          hint={countHint(folder.meeting_count)}
          selected={selection === folder.id}
          onPick={() => onPick(folder.id)}
        >
          {abbreviation(folder.name) || (
            <FolderIcon width={16} height={16} aria-hidden="true" />
          )}
        </RailItem>
      ))}
      <RailItem
        id={NO_PROJECT}
        label={t("meetings.projects.none")}
        hint={countHint(counts.none)}
        selected={selection === NO_PROJECT}
        onPick={() => onPick(NO_PROJECT)}
      >
        <FolderMinus width={16} height={16} aria-hidden="true" />
      </RailItem>
    </div>
  );
};
