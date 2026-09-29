import React from "react";
import { useTranslation } from "react-i18next";
import type { ActionItem, EnhancedEntry, EnhancedSection } from "@/bindings";

interface TaskChecklistProps {
  section: EnhancedSection;
  /** Aufgabe je Eintrag (`entry_id`) der angezeigten Version. */
  itemOf: (entry: EnhancedEntry) => ActionItem | undefined;
  onToggle: (item: ActionItem, done: boolean) => void;
  /** Aeltere Versionen sind nur zum Ansehen: ihre Aufgaben gehoeren nicht mehr zur Liste. */
  readOnly: boolean;
  /** Text und Quellen des Eintrags (gleiche Darstellung wie in Textabschnitten). */
  renderEntry: (entry: EnhancedEntry, done: boolean) => React.ReactNode;
}

/**
 * Aufgabenabschnitt als Checkliste: die Checkbox setzt den Status der
 * Aufgabe (`action_items_set_status`), Verantwortliche und Termin stehen
 * gedaempft daneben.
 */
export const TaskChecklist: React.FC<TaskChecklistProps> = ({
  section,
  itemOf,
  onToggle,
  readOnly,
  renderEntry,
}) => {
  const { t } = useTranslation();
  return (
    <ul className="space-y-1" data-testid="task-checklist">
      {section.entries.map((entry) => {
        const item = itemOf(entry);
        const done = item?.status === "done";
        const meta = [
          entry.assignee ? `@${entry.assignee}` : null,
          entry.due ? t("meetings.enhanced.due", { due: entry.due }) : null,
        ].filter(Boolean);
        return (
          <li
            key={entry.id}
            data-task-entry={entry.id}
            className="flex items-start gap-2"
          >
            <input
              type="checkbox"
              checked={done}
              disabled={readOnly || !item}
              onChange={(e) => item && onToggle(item, e.target.checked)}
              aria-label={t("meetings.enhanced.taskDone")}
              className="mt-1.5 shrink-0 accent-logo-primary"
            />
            <div className="min-w-0 flex-1">
              {renderEntry(entry, done)}
              {meta.length > 0 && (
                <p className="text-xs text-text/50">{meta.join(" · ")}</p>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
};
