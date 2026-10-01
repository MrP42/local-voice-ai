import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { MessageSquare, Plus, Users, X } from "lucide-react";
import {
  commands,
  type Meeting,
  type MeetingSpeaker,
  type Participant,
  type PersonSummary,
} from "@/bindings";
import { initials, orderParticipants } from "@/lib/meetingPeople";
import { Button } from "../../ui/Button";
import { IconAction } from "../../ui/IconAction";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { metadataErrorKey } from "./meetingErrors";
import { PersonNameField } from "./PersonNameField";
import type { PersonRef } from "./people/PersonPopover";
import { speakerErrorText } from "./SpeakerPopover";

const PANEL_WIDTH = 336;
const MARGIN = 8;
/** Der Chip nennt hoechstens so viele Namen; der Rest steht im Tooltip. */
const CHIP_NAMES = 3;

const fold = (value: string) =>
  value.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();

const speakerKey = (s: Pick<MeetingSpeaker, "channel" | "speaker_index">) =>
  `${s.channel}:${s.speaker_index}`;

interface ParticipantsPopoverProps {
  meeting: Meeting;
  participants: Participant[];
  /** Erkannte Sprecher (zum Verknuepfen eines Namens). */
  speakers: MeetingSpeaker[];
  /** Die Metadaten wurden gespeichert (Teilnehmende geaendert). */
  onSaved: (meeting: Meeting) => void;
  /** Ein Sprecher wurde benannt (Segmente und Personen neu lesen). */
  onSpeakersChanged: () => void;
  onFilter: (person: PersonRef) => void;
  onAsk: (person: PersonRef) => void;
  onManage: () => void;
}

/**
 * Teilnehmende im Kopf der Besprechung (G4, #70): ein Chip "N Teilnehmende:
 * Name, Name, ..." (gekuerzt, voll im Tooltip). Ein Klick oeffnet ein Popover:
 * Personen entfernen, Personen hinzufuegen (Autovervollstaendigung aus den
 * bekannten Personen) und einen Namen mit einem erkannten Sprecher verknuepfen.
 * Gespeichert wird ueber dieselben Befehle wie im Details-Dialog
 * (`meetings_update_metadata`); die Verknuepfung mit einem Sprecher benennt ihn
 * (`meeting_speaker_rename`), die Person entsteht daraus.
 */
export const ParticipantsPopover: React.FC<ParticipantsPopoverProps> = ({
  meeting,
  participants,
  speakers,
  onSaved,
  onSpeakersChanged,
  onFilter,
  onAsk,
  onManage,
}) => {
  const { t } = useTranslation();
  const tip = `${useId()}-tip`;
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [name, setName] = useState("");
  const [picked, setPicked] = useState<PersonSummary | null>(null);
  const [speaker, setSpeaker] = useState("");
  const [known, setKnown] = useState<PersonSummary[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const ordered = useMemo(
    () => orderParticipants(participants),
    [participants],
  );
  const label = (p: Participant) =>
    p.is_self ? `${p.name} (${t("meetings.people.you")})` : p.name;
  const shown = ordered.slice(0, CHIP_NAMES).map(label).join(", ");
  const more = ordered.length - CHIP_NAMES;
  const names =
    shown +
    (more > 0 ? `, ${t("meetings.header.peopleMore", { count: more })}` : "");
  const fullList = ordered.map(label).join(", ");

  const place = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const height = panelRef.current?.offsetHeight ?? 0;
    const left = Math.max(
      MARGIN,
      Math.min(rect.left, window.innerWidth - PANEL_WIDTH - MARGIN),
    );
    const below = rect.bottom + 4;
    const top =
      height > 0 && below + height > window.innerHeight - MARGIN
        ? Math.max(MARGIN, rect.top - height - 4)
        : below;
    setPos({ left, top });
  }, []);

  const close = useCallback(() => setOpen(false), []);

  // Beim Oeffnen: bekannte Personen holen, Eingabe leeren.
  useEffect(() => {
    if (!open) return;
    setName("");
    setPicked(null);
    setSpeaker("");
    setError(null);
    let alive = true;
    void commands
      .peopleList(null)
      .then((result) => {
        if (alive && result.status === "ok") setKnown(result.data ?? []);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open, place, ordered.length, error, speakers.length]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        panelRef.current?.contains(target) ||
        triggerRef.current?.contains(target)
      )
        return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        close();
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close]);

  const ids = participants.map((p) => p.human_id);

  /** Die Person, die der Name meint: gewaehlt oder genau so bekannt. */
  const target =
    picked && fold(picked.name) === fold(name)
      ? picked
      : (known.find((p) => fold(p.name) === fold(name)) ?? null);
  const already = target !== null && ids.includes(target.id);
  const trimmed = name.trim();
  const linking = speaker !== "";
  const canAdd =
    !busy && trimmed !== "" && (linking || (target !== null && !already));

  const saveParticipants = async (next: string[]) => {
    setBusy(true);
    setError(null);
    try {
      const result = await commands.meetingsUpdateMetadata(meeting.id, {
        title: null,
        description: null,
        started_at: null,
        participant_ids: next,
        folder_ids: null,
      });
      if (result.status === "error") {
        setError(
          t(metadataErrorKey(result.error), { defaultValue: result.error }),
        );
        return false;
      }
      onSaved(result.data);
      return true;
    } finally {
      setBusy(false);
    }
  };

  const remove = (person: Participant) =>
    void saveParticipants(ids.filter((id) => id !== person.human_id));

  const add = async () => {
    if (!canAdd) return;
    if (linking) {
      const chosen = speakers.find((s) => speakerKey(s) === speaker);
      if (!chosen) return;
      setBusy(true);
      setError(null);
      const result = await commands.meetingSpeakerRename(
        meeting.id,
        chosen.channel,
        chosen.speaker_index,
        trimmed,
      );
      setBusy(false);
      if (result.status === "error") {
        setError(speakerErrorText(result.error, t));
        return;
      }
      onSpeakersChanged();
    } else if (target) {
      if (!(await saveParticipants([...ids, target.id]))) return;
    }
    setName("");
    setPicked(null);
    setSpeaker("");
  };

  const run = (action: () => void) => {
    close();
    action();
  };

  return (
    <>
      <TooltipTrigger
        tooltipId={tip}
        className="inline-flex min-w-0 shrink"
        content={
          <>
            <div className="text-sm font-semibold">
              <strong>
                {participants.length > 0
                  ? t("meetings.header.participants", {
                      count: participants.length,
                    })
                  : t("meetings.header.participantsNone")}
              </strong>
            </div>
            <div className="text-xs text-text/70">
              {participants.length > 0
                ? fullList
                : t("meetings.header.participantsHint")}
            </div>
          </>
        }
      >
        <button
          ref={triggerRef}
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-describedby={tip}
          data-testid="participants-chip"
          data-count={participants.length}
          className={`inline-flex h-6 min-w-0 shrink cursor-pointer items-center gap-1 rounded-full border px-2 text-xs transition-colors focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary ${
            open
              ? "border-logo-primary bg-logo-primary/15 text-text"
              : "border-mid-gray/30 text-text/80 hover:border-logo-primary"
          }`}
        >
          <Users
            width={12}
            height={12}
            aria-hidden="true"
            className="shrink-0"
          />
          {participants.length > 0 ? (
            <>
              <span className="shrink-0 whitespace-nowrap">
                {t("meetings.header.participants", {
                  count: participants.length,
                })}
                :
              </span>
              <span
                className="min-w-0 truncate"
                data-testid="participants-names"
              >
                {names}
              </span>
            </>
          ) : (
            <span className="truncate">
              {t("meetings.header.participantsNone")}
            </span>
          )}
        </button>
      </TooltipTrigger>
      {open &&
        createPortal(
          <div
            ref={panelRef}
            role="dialog"
            aria-label={t("meetings.header.participantsTitle")}
            data-testid="participants-popover"
            style={{
              position: "fixed",
              left: pos?.left ?? -9999,
              top: pos?.top ?? -9999,
              width: PANEL_WIDTH,
            }}
            className="z-50 space-y-2 rounded-lg border border-mid-gray/30 bg-background p-3 text-sm shadow-lg"
          >
            <p className="font-semibold">
              {t("meetings.header.participantsTitle")}
            </p>
            {ordered.length === 0 ? (
              <p className="text-xs text-text/60">
                {t("meetings.header.participantsEmpty")}
              </p>
            ) : (
              <ul
                className="max-h-56 space-y-1 overflow-y-auto"
                data-testid="participants-list"
              >
                {ordered.map((person) => {
                  const ref: PersonRef = {
                    id: person.human_id,
                    name: person.name,
                  };
                  return (
                    <li
                      key={person.human_id}
                      data-testid="participant-row"
                      data-human-id={person.human_id}
                      data-role={person.role}
                      className="flex items-center gap-2 rounded-md border border-mid-gray/20 px-2 py-1"
                    >
                      <span
                        aria-hidden="true"
                        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-mid-gray/25 text-[10px] font-semibold"
                      >
                        {initials(person.name)}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span
                          className="block truncate font-medium"
                          data-testid="participant-name"
                        >
                          {person.name}
                          {person.is_self && (
                            <span className="font-normal text-text/50">
                              {" "}
                              ({t("meetings.people.you")})
                            </span>
                          )}
                        </span>
                        <span className="block truncate text-xs text-text/60">
                          {t(`meetings.people.role.${person.role}`, {
                            defaultValue: person.role,
                          })}
                          {" · "}
                          {t(`meetings.people.source.${person.source}`, {
                            defaultValue: person.source,
                          })}
                          {person.email ? ` · ${person.email}` : ""}
                          {person.company ? ` · ${person.company}` : ""}
                        </span>
                      </span>
                      <IconAction
                        ghost
                        size="sm"
                        icon={Users}
                        label={t("meetings.people.popover.meetingsWith", {
                          count: person.meeting_count,
                          name: person.name,
                        })}
                        description={t(
                          "meetings.people.popover.meetingsFilterTitle",
                          { name: person.name },
                        )}
                        testId="person-meetings"
                        disabled={person.meeting_count === 0}
                        onClick={() => run(() => onFilter(ref))}
                      />
                      <IconAction
                        ghost
                        size="sm"
                        icon={MessageSquare}
                        label={t("meetings.people.popover.ask")}
                        description={t("meetings.people.popover.askTitle", {
                          name: person.name,
                        })}
                        testId="person-ask"
                        disabled={person.meeting_count === 0}
                        onClick={() => run(() => onAsk(ref))}
                      />
                      <IconAction
                        ghost
                        size="sm"
                        icon={X}
                        label={t("meetings.metadata.removePerson", {
                          name: person.name,
                        })}
                        description={t("meetings.header.participantRemoveHint")}
                        testId="participant-remove"
                        disabled={busy}
                        onClick={() => remove(person)}
                      />
                    </li>
                  );
                })}
              </ul>
            )}

            <form
              className="space-y-1.5 border-t border-mid-gray/20 pt-2"
              data-testid="participants-add"
              onSubmit={(e) => {
                e.preventDefault();
                void add();
              }}
            >
              <PersonNameField
                value={name}
                onChange={(value) => {
                  setName(value);
                  setPicked(null);
                }}
                onPick={(person) => {
                  setName(person.name);
                  setPicked(person);
                }}
                placeholder={t("meetings.header.participantAddPlaceholder")}
                ariaLabel={t("meetings.header.participantAddPlaceholder")}
                inputTestId="participant-add-input"
                autoFocus
                disabled={busy}
                exclude={participants.map((p) => p.name)}
              />
              {speakers.length > 0 && (
                <div
                  role="group"
                  aria-label={t("meetings.header.participantSpeaker")}
                  data-testid="participant-speakers"
                  className="flex flex-wrap items-center gap-1 text-xs"
                >
                  <span className="text-text/60">
                    {t("meetings.header.participantSpeaker")}
                  </span>
                  {speakers.map((s) => {
                    const on = speaker === speakerKey(s);
                    return (
                      <button
                        key={speakerKey(s)}
                        type="button"
                        aria-pressed={on}
                        disabled={busy}
                        data-testid="participant-speaker"
                        data-speaker={speakerKey(s)}
                        title={t("meetings.header.participantSpeakerHint", {
                          label: s.label,
                        })}
                        onClick={() => setSpeaker(on ? "" : speakerKey(s))}
                        className={`h-6 max-w-full cursor-pointer truncate rounded-full border px-2 transition-colors focus:outline-none focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-logo-primary ${
                          on
                            ? "border-logo-primary bg-logo-primary/20 text-text"
                            : "border-mid-gray/40 text-text/80 hover:border-logo-primary"
                        }`}
                      >
                        {s.label}
                      </button>
                    );
                  })}
                </div>
              )}
              {trimmed !== "" && !linking && target === null && (
                <p
                  className="text-xs text-text/60"
                  data-testid="participant-unknown"
                >
                  {speakers.length > 0
                    ? t("meetings.header.participantUnknownSpeaker")
                    : t("meetings.header.participantUnknown")}
                </p>
              )}
              {already && !linking && (
                <p className="text-xs text-text/60">
                  {t("meetings.header.participantAlready")}
                </p>
              )}
              {error && (
                <p
                  className="text-xs text-red-400"
                  role="alert"
                  data-testid="participants-error"
                >
                  {error}
                </p>
              )}
              <div className="flex items-center justify-between gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  type="button"
                  onClick={() => run(onManage)}
                  data-testid="person-manage"
                >
                  {t("meetings.people.popover.manage")}
                </Button>
                <Button
                  size="sm"
                  type="submit"
                  disabled={!canAdd}
                  data-testid="participant-add"
                >
                  <Plus width={12} height={12} aria-hidden="true" />
                  {t("meetings.header.participantAdd")}
                </Button>
              </div>
            </form>
          </div>,
          document.body,
        )}
    </>
  );
};
