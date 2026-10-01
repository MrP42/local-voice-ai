import React from "react";
import { useTranslation } from "react-i18next";

/** Herkunft der Besprechung (`meetings.source`), nicht die Trefferquelle. */
export type MeetingSourceFilter = "live" | "import" | "subtitle";

export interface ListFilter {
  /** Zeitraum rueckwaerts ab jetzt in Tagen; `null` = ohne Grenze. */
  rangeDays: number | null;
  source: MeetingSourceFilter | null;
  hasNotes: boolean;
}

export const EMPTY_FILTER: ListFilter = {
  rangeDays: null,
  source: null,
  hasNotes: false,
};

const RANGES: { days: number; key: string }[] = [
  { days: 7, key: "meetings.search.range7" },
  { days: 30, key: "meetings.search.range30" },
  { days: 365, key: "meetings.search.range365" },
];

const SOURCES: { value: MeetingSourceFilter; key: string }[] = [
  { value: "live", key: "meetings.search.sourceLive" },
  { value: "import", key: "meetings.search.sourceImport" },
  { value: "subtitle", key: "meetings.search.sourceSubtitle" },
];

/** Gemeinsamer Chip-Stil fuer Filter- und Ordner-Chips. */
export const chipClass = (active: boolean) =>
  `inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 ${
    active
      ? "bg-logo-primary/20 border-logo-primary text-text"
      : "border-mid-gray/40 text-text/70 hover:bg-mid-gray/15 hover:text-text"
  }`;

interface FilterChipsProps {
  value: ListFilter;
  onChange: (value: ListFilter) => void;
}

/** Zeitraum und Herkunft schliessen sich innerhalb ihrer Gruppe aus; ein
 *  zweiter Klick auf den aktiven Chip hebt ihn auf. */
export const FilterChips: React.FC<FilterChipsProps> = ({
  value,
  onChange,
}) => {
  const { t } = useTranslation();
  return (
    <div
      className="flex flex-wrap items-center gap-1.5"
      role="group"
      aria-label={t("meetings.search.filters")}
    >
      {RANGES.map((r) => {
        const active = value.rangeDays === r.days;
        return (
          <button
            key={r.days}
            type="button"
            aria-pressed={active}
            className={chipClass(active)}
            onClick={() =>
              onChange({ ...value, rangeDays: active ? null : r.days })
            }
          >
            {t(r.key)}
          </button>
        );
      })}
      <span className="mx-0.5 h-4 w-px bg-mid-gray/30" aria-hidden="true" />
      {SOURCES.map((s) => {
        const active = value.source === s.value;
        return (
          <button
            key={s.value}
            type="button"
            aria-pressed={active}
            className={chipClass(active)}
            onClick={() =>
              onChange({ ...value, source: active ? null : s.value })
            }
          >
            {t(s.key)}
          </button>
        );
      })}
      <span className="mx-0.5 h-4 w-px bg-mid-gray/30" aria-hidden="true" />
      <button
        type="button"
        aria-pressed={value.hasNotes}
        className={chipClass(value.hasNotes)}
        onClick={() => onChange({ ...value, hasNotes: !value.hasNotes })}
      >
        {t("meetings.search.hasNotes")}
      </button>
    </div>
  );
};
