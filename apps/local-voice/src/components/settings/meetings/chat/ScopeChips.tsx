import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { commands, type Folder, type ScopeFilter } from "@/bindings";
import { formatDay } from "@/lib/meetingChat";

export const EMPTY_SCOPE: ScopeFilter = {
  meeting_ids: null,
  folder_id: null,
  person: null,
  person_id: null,
  from: null,
  to: null,
  event_uid: null,
};

interface ScopeChipsProps {
  filter: ScopeFilter;
  folders: Folder[];
  /** Ohne Rueckruf sind die Chips nur Anzeige. */
  onChange?: (filter: ScopeFilter) => void;
}

/**
 * Eingrenzung eines Chats ueber viele Besprechungen als Chips (Ordner,
 * Zeitraum, Person, Auswahl). Ein Chip laesst sich entfernen; ohne Chip gilt
 * "Alle Besprechungen".
 */
export const ScopeChips: React.FC<ScopeChipsProps> = ({
  filter,
  folders,
  onChange,
}) => {
  const { t, i18n } = useTranslation();
  // M5-P5d: eine bekannte Person steht als ID im Scope, der Chip zeigt ihren Namen.
  const [personName, setPersonName] = useState<{
    id: string;
    name: string;
  } | null>(null);
  useEffect(() => {
    const id = filter.person_id;
    if (!id) return;
    let cancelled = false;
    void commands.peopleGet(id).then((result) => {
      if (!cancelled && result.status === "ok") {
        setPersonName({ id, name: result.data.name });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [filter.person_id]);
  const chips: { key: keyof ScopeFilter; label: string }[] = [];
  if (filter.meeting_ids && filter.meeting_ids.length > 0) {
    chips.push({
      key: "meeting_ids",
      label: t("meetings.chat.scopeChips.selection", {
        count: filter.meeting_ids.length,
      }),
    });
  }
  if (filter.folder_id) {
    const name =
      folders.find((f) => f.id === filter.folder_id)?.name ?? filter.folder_id;
    chips.push({
      key: "folder_id",
      label: t("meetings.chat.scopeChips.folder", { name }),
    });
  }
  if (filter.person) {
    chips.push({
      key: "person",
      label: t("meetings.chat.scopeChips.person", { name: filter.person }),
    });
  }
  if (filter.person_id) {
    const name = personName?.id === filter.person_id ? personName.name : "…";
    chips.push({
      key: "person_id",
      label: t("meetings.chat.scopeChips.person", { name }),
    });
  }
  if (filter.from !== null) {
    chips.push({
      key: "from",
      label: t("meetings.chat.scopeChips.from", {
        date: formatDay(filter.from, i18n.language),
      }),
    });
  }
  if (filter.to !== null) {
    chips.push({
      key: "to",
      label: t("meetings.chat.scopeChips.to", {
        date: formatDay(filter.to, i18n.language),
      }),
    });
  }

  const chipClass =
    "inline-flex items-center gap-1 rounded-full border border-logo-primary/60 bg-logo-primary/15 px-2.5 py-0.5 text-xs text-text";

  return (
    <div
      role="group"
      aria-label={t("meetings.chat.scopeChips.label")}
      className="flex flex-wrap items-center gap-1.5"
    >
      {chips.length === 0 ? (
        <span className={chipClass}>{t("meetings.chat.scopeChips.all")}</span>
      ) : (
        chips.map((chip) => (
          <span key={chip.key} className={chipClass}>
            {chip.label}
            {onChange && (
              <button
                type="button"
                aria-label={t("meetings.chat.scopeChips.remove", {
                  label: chip.label,
                })}
                title={t("meetings.chat.scopeChips.remove", {
                  label: chip.label,
                })}
                onClick={() => onChange({ ...filter, [chip.key]: null })}
                className="rounded-full p-0.5 text-text/60 hover:bg-mid-gray/20 hover:text-text cursor-pointer"
              >
                <X width={10} height={10} aria-hidden="true" />
              </button>
            )}
          </span>
        ))
      )}
    </div>
  );
};
