import React, { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Filter } from "lucide-react";
import { ActionPopover } from "../../../ui/ActionMenu";
import {
  EMPTY_FILTER,
  FilterChips,
  type ListFilter,
} from "../search/FilterChips";
import { Button } from "../../../ui/Button";

/** Wie viele der Filter (Zeitraum, Quelle, Notizen) gerade gesetzt sind. */
export const activeFilterCount = (filter: ListFilter): number =>
  (filter.rangeDays !== null ? 1 : 0) +
  (filter.source !== null ? 1 : 0) +
  (filter.hasNotes ? 1 : 0);

interface ProjectFilterProps {
  value: ListFilter;
  onChange: (value: ListFilter) => void;
}

/**
 * Zeitraum, Quelle und "Mit Notizen" hinter einem Symbol neben der Suche. Die
 * Zahl am Symbol sagt, dass die Liste gefiltert ist, auch wenn das Fenster zu ist.
 */
export const ProjectFilter: React.FC<ProjectFilterProps> = ({
  value,
  onChange,
}) => {
  const { t } = useTranslation();
  const count = activeFilterCount(value);
  const chipsRef = useRef<HTMLDivElement>(null);
  return (
    <ActionPopover
      trigger={{
        icon: Filter,
        label: t("meetings.projects.filter.label"),
        description: t("meetings.projects.filter.hint"),
        testId: "projects-filter",
        size: "sm",
        badge: count > 0 ? count : undefined,
        badgeTestId: "projects-filter-count",
      }}
      popoverLabel={t("meetings.projects.filter.label")}
      popoverTestId="projects-filter-popover"
    >
      <div ref={chipsRef}>
        <FilterChips value={value} onChange={onChange} />
      </div>
      {count > 0 && (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            onChange(EMPTY_FILTER);
            // Der Knopf verschwindet: der Fokus bleibt im Fenster.
            chipsRef.current?.querySelector("button")?.focus();
          }}
        >
          {t("meetings.projects.filter.reset")}
        </Button>
      )}
    </ActionPopover>
  );
};
