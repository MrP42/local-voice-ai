import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import {
  commands,
  type Folder,
  type Meeting,
  type MetadataEdit,
  type Participant,
  type PersonSummary,
} from "@/bindings";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { Textarea } from "../../ui/Textarea";
import { metadataErrorKey } from "./meetingErrors";

/** Laengste Beschreibung (wie im Backend, `metadata::DESCRIPTION_MAX_CHARS`). */
export const DESCRIPTION_MAX_CHARS = 4_000;
/** Laengster Titel (`metadata::TITLE_MAX_CHARS`). */
export const TITLE_MAX_CHARS = 300;
/** So viele Personen schlaegt die Suche hoechstens vor. */
const SUGGESTIONS = 8;

const pad = (n: number) => n.toString().padStart(2, "0");

/** Unix-Sekunden als Wert eines `datetime-local`-Feldes (Ortszeit, Minuten). */
export const toLocalInput = (seconds: number): string => {
  const d = new Date(seconds * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

/** Wert eines `datetime-local`-Feldes als Unix-Sekunden (Ortszeit); `null`, wenn leer oder ungueltig. */
export const fromLocalInput = (value: string): number | null => {
  if (!value) return null;
  const ms = new Date(value).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
};

const sameSet = (a: string[], b: string[]) =>
  a.length === b.length && a.every((x) => b.includes(x));

/** Was sich gegenueber der Besprechung geaendert hat; alles andere bleibt `null` (unveraendert). */
export const buildEdit = (
  meeting: Pick<Meeting, "title" | "description" | "started_at" | "created_at">,
  draft: {
    title: string;
    description: string;
    datetime: string;
    participantIds: string[];
    folderIds: string[];
  },
  original: { participantIds: string[]; folderIds: string[] },
): MetadataEdit => {
  const title = draft.title.trim();
  const description = draft.description.trim();
  const originalWhen = toLocalInput(meeting.started_at ?? meeting.created_at);
  return {
    title: title !== meeting.title ? title : null,
    description:
      description !== (meeting.description ?? "").trim() ? description : null,
    started_at:
      draft.datetime !== originalWhen ? fromLocalInput(draft.datetime) : null,
    participant_ids: sameSet(draft.participantIds, original.participantIds)
      ? null
      : draft.participantIds,
    folder_ids: sameSet(draft.folderIds, original.folderIds)
      ? null
      : draft.folderIds,
  };
};

const isEmptyEdit = (edit: MetadataEdit) =>
  Object.values(edit).every((value) => value === null);

interface MeetingMetadataFormProps {
  meeting: Meeting;
  /** Die Teilnehmenden jetzt. */
  participants: Participant[];
  /** Alle Projekte und die, in denen die Besprechung liegt. */
  folders: Folder[];
  folderIds: string[];
  onSaved: (meeting: Meeting) => void;
  onCancel: () => void;
}

/**
 * Bearbeiten-Modus des Details-Dialogs (U7): Titel, Beschreibung (mehrzeilig),
 * Datum/Uhrzeit, Teilnehmende (vorhandene Personen) und Projekte (n:m). Dateiname
 * und Quelle bleiben unveraendert und stehen im Dialog darueber. Gespeichert wird
 * alles oder nichts; bei einem Fehler bleibt der Dialog mit der Eingabe offen.
 */
export const MeetingMetadataForm: React.FC<MeetingMetadataFormProps> = ({
  meeting,
  participants,
  folders,
  folderIds,
  onSaved,
  onCancel,
}) => {
  const { t } = useTranslation();
  const originalPeople = useMemo(
    () => participants.map((p) => p.human_id),
    [participants],
  );
  const [title, setTitle] = useState(meeting.title);
  const [description, setDescription] = useState(meeting.description ?? "");
  const [datetime, setDatetime] = useState(
    toLocalInput(meeting.started_at ?? meeting.created_at),
  );
  // Ausgewaehlte Personen samt Namen (auch solche, die die Liste nicht kennt).
  const [people, setPeople] = useState<
    { id: string; name: string; email: string | null }[]
  >(
    participants.map((p) => ({
      id: p.human_id,
      name: p.name,
      email: p.email,
    })),
  );
  const [known, setKnown] = useState<PersonSummary[]>([]);
  const [query, setQuery] = useState("");
  const [selectedFolders, setSelectedFolders] = useState<string[]>(folderIds);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void commands.peopleList(null).then((result) => {
      if (alive && result.status === "ok") setKnown(result.data ?? []);
    });
    return () => {
      alive = false;
    };
  }, []);

  const selectedIds = people.map((p) => p.id);
  const needle = query.trim().toLowerCase();
  const suggestions = known
    .filter((p) => !selectedIds.includes(p.id))
    .filter(
      (p) =>
        needle === "" ||
        [p.name, p.email ?? "", p.company ?? ""].some((field) =>
          field.toLowerCase().includes(needle),
        ),
    )
    .slice(0, SUGGESTIONS);

  const edit = buildEdit(
    meeting,
    {
      title,
      description,
      datetime,
      participantIds: selectedIds,
      folderIds: selectedFolders,
    },
    { participantIds: originalPeople, folderIds },
  );
  const titleEmpty = title.trim() === "";
  const dateInvalid = fromLocalInput(datetime) === null;
  const tooLong = description.trim().length > DESCRIPTION_MAX_CHARS;
  const dirty = !isEmptyEdit(edit);
  const canSave = dirty && !titleEmpty && !dateInvalid && !tooLong && !saving;

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      const result = await commands.meetingsUpdateMetadata(meeting.id, edit);
      if (result.status === "error") {
        setError(
          t(metadataErrorKey(result.error), { defaultValue: result.error }),
        );
        return;
      }
      onSaved(result.data);
    } finally {
      setSaving(false);
    }
  };

  const toggleFolder = (id: string) =>
    setSelectedFolders((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id],
    );

  return (
    <form
      data-testid="meta-form"
      className="space-y-3 text-sm"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label className="block space-y-1">
        <span className="block text-text/60">
          {t("meetings.metadata.title")}
        </span>
        <Input
          data-testid="meta-title"
          className="w-full"
          value={title}
          maxLength={TITLE_MAX_CHARS}
          onChange={(e) => setTitle(e.target.value)}
        />
        {titleEmpty && (
          <span className="block text-xs text-red-400">
            {t("meetings.metadata.errors.titleEmpty")}
          </span>
        )}
      </label>

      <label className="block space-y-1">
        <span className="block text-text/60">
          {t("meetings.metadata.description")}
        </span>
        <Textarea
          data-testid="meta-description"
          className="w-full"
          rows={4}
          value={description}
          placeholder={t("meetings.metadata.descriptionPlaceholder")}
          onChange={(e) => setDescription(e.target.value)}
        />
        <span
          className={`block text-xs ${tooLong ? "text-red-400" : "text-text/50"}`}
          data-testid="meta-description-count"
        >
          {t("meetings.metadata.descriptionCount", {
            count: description.trim().length,
            max: DESCRIPTION_MAX_CHARS,
          })}
        </span>
      </label>

      <label className="block space-y-1">
        <span className="block text-text/60">
          {t("meetings.metadata.datetime")}
        </span>
        <Input
          data-testid="meta-datetime"
          type="datetime-local"
          value={datetime}
          onChange={(e) => setDatetime(e.target.value)}
        />
        {dateInvalid && (
          <span className="block text-xs text-red-400">
            {t("meetings.metadata.errors.dateInvalid")}
          </span>
        )}
      </label>

      <fieldset className="space-y-1.5">
        <legend className="text-text/60">
          {t("meetings.metadata.participants")}
        </legend>
        <ul
          className="flex flex-wrap gap-1.5"
          data-testid="meta-people"
          aria-label={t("meetings.metadata.participants")}
        >
          {people.length === 0 && (
            <li className="text-xs text-text/50">
              {t("meetings.metadata.participantsNone")}
            </li>
          )}
          {people.map((person) => (
            <li
              key={person.id}
              data-testid={`meta-person-chip-${person.id}`}
              className="inline-flex items-center gap-1 rounded-full border border-mid-gray/40 bg-mid-gray/10 ps-2 pe-1 text-xs"
            >
              {person.name}
              <button
                type="button"
                data-testid={`meta-person-remove-${person.id}`}
                aria-label={t("meetings.metadata.removePerson", {
                  name: person.name,
                })}
                className="rounded-full p-0.5 hover:bg-mid-gray/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
                onClick={() =>
                  setPeople((prev) => prev.filter((p) => p.id !== person.id))
                }
              >
                <X width={12} height={12} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
        <Input
          data-testid="meta-people-search"
          className="w-full"
          value={query}
          placeholder={t("meetings.metadata.participantsSearch")}
          aria-label={t("meetings.metadata.participantsSearch")}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            // Enter in der Suche waehlt den ersten Vorschlag, statt zu speichern.
            if (e.key === "Enter") {
              e.preventDefault();
              const first = suggestions[0];
              if (first) {
                setPeople((prev) => [
                  ...prev,
                  { id: first.id, name: first.name, email: first.email },
                ]);
                setQuery("");
              }
            }
          }}
        />
        <ul className="flex flex-wrap gap-1.5" data-testid="meta-suggestions">
          {suggestions.length === 0 && needle !== "" && (
            <li className="text-xs text-text/50">
              {t("meetings.metadata.noPeopleFound")}
            </li>
          )}
          {suggestions.map((person) => (
            <li key={person.id}>
              <button
                type="button"
                data-testid={`meta-person-add-${person.id}`}
                title={person.email ?? undefined}
                className="rounded-full border border-dashed border-mid-gray/50 px-2 text-xs hover:border-logo-primary hover:bg-logo-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
                onClick={() => {
                  setPeople((prev) => [
                    ...prev,
                    { id: person.id, name: person.name, email: person.email },
                  ]);
                  setQuery("");
                }}
              >
                + {person.name}
              </button>
            </li>
          ))}
        </ul>
      </fieldset>

      <fieldset className="space-y-1">
        <legend className="text-text/60">
          {t("meetings.metadata.projects")}
        </legend>
        {folders.length === 0 ? (
          <p className="text-xs text-text/50">
            {t("meetings.metadata.noProjectsYet")}
          </p>
        ) : (
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            {folders.map((folder) => (
              <label
                key={folder.id}
                className="flex cursor-pointer items-center gap-1.5"
              >
                <input
                  type="checkbox"
                  data-testid={`meta-project-${folder.id}`}
                  checked={selectedFolders.includes(folder.id)}
                  onChange={() => toggleFolder(folder.id)}
                />
                {folder.name}
              </label>
            ))}
          </div>
        )}
      </fieldset>

      <p className="text-xs text-text/50">{t("meetings.metadata.fixedHint")}</p>

      {error && (
        <div data-testid="meta-error">
          <Alert variant="error">{error}</Alert>
        </div>
      )}

      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="secondary"
          data-testid="meta-cancel"
          onClick={onCancel}
        >
          {t("meetings.metadata.cancel")}
        </Button>
        <Button type="submit" data-testid="meta-save" disabled={!canSave}>
          {saving ? t("meetings.metadata.saving") : t("meetings.metadata.save")}
        </Button>
      </div>
    </form>
  );
};
