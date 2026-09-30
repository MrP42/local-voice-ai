import React from "react";
import { useTranslation } from "react-i18next";
import type { Meeting } from "@/bindings";
import { NOTES_MAX_BYTES } from "@/lib/meetingNotes";
import { Alert } from "../../../ui/Alert";
import { NoteBlocksEditor } from "./NoteBlocksEditor";
import { MeetingTemplatePicker } from "./TemplatePicker";
import {
  recordingStamper,
  useNotesAutosave,
  type NotesAutosave,
} from "./useNotesAutosave";

/**
 * Speicherstatus, Groessenwarnung und Konflikthinweis eines Notizblocks;
 * gemeinsam fuer laufende Aufnahme und Detailansicht (beides `MyNotesView`).
 */
export const NotesStatusLine: React.FC<{ autosave: NotesAutosave }> = ({
  autosave,
}) => {
  const { t } = useTranslation();
  const { status, conflicts, loadError } = autosave;
  const label =
    status === "saving"
      ? t("meetings.notes.status.saving")
      : status === "dirty"
        ? t("meetings.notes.status.dirty")
        : status === "saved"
          ? t("meetings.notes.status.saved")
          : status === "error"
            ? t("meetings.notes.status.error")
            : "";
  return (
    <>
      {loadError && (
        <Alert variant="error">
          {t("meetings.notes.loadError", { error: loadError })}
        </Alert>
      )}
      {status === "tooLarge" && (
        <Alert variant="warning">
          {t("meetings.notes.tooLarge", {
            limit: Math.round(NOTES_MAX_BYTES / 1024),
          })}
        </Alert>
      )}
      {conflicts > 0 && (
        <p className="text-xs text-text/60" data-testid="notes-conflict">
          {t("meetings.notes.conflict")}
        </p>
      )}
      <p
        aria-live="polite"
        data-testid="notes-status"
        data-status={status}
        className={`text-xs ${status === "error" ? "text-red-400" : "text-text/50"}`}
      >
        {label}
      </p>
    </>
  );
};

/**
 * "Meine Notizen" in der Besprechungsdetailansicht: derselbe Block-Editor und
 * derselbe Autosave fuer Live und danach. Laeuft die Aufnahme dieser
 * Besprechung (`live`), hat der Editor den Fokus, tragen neue Bloecke die
 * Audioposition, und die Vorlage laesst sich gleich hier waehlen; nach dem
 * Stopp und bei importierten Besprechungen bleiben neue Bloecke ohne
 * Zeitstempel. `compact` (schmales Fenster) spart Hinweis und Vorlagenwahl:
 * die Vorlage steht dann im Menue.
 */
export const MyNotesView: React.FC<{
  meeting: Meeting;
  live?: boolean;
  compact?: boolean;
}> = ({ meeting, live = false, compact = false }) => {
  const { t } = useTranslation();
  const autosave = useNotesAutosave(meeting.id);
  const stamp = React.useMemo(
    () => (live ? recordingStamper(meeting.id) : undefined),
    [meeting.id, live],
  );

  // Der Fokus gehoert dem Notizfeld, sobald die Aufnahme dieser Besprechung
  // laeuft und der Block geladen ist (`autoFocus` allein reicht nicht: `live`
  // wird oft erst nach dem Laden wahr). Nur einmal je Aufnahme und nur, wenn
  // sonst niemand den Fokus hat (Nutzer tippt schon woanders, Dialog offen).
  const rootRef = React.useRef<HTMLDivElement>(null);
  const focusedFor = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!live || !autosave.loaded || focusedFor.current === meeting.id) return;
    focusedFor.current = meeting.id;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    const fields = rootRef.current?.querySelectorAll("textarea");
    fields?.[fields.length - 1]?.focus({ preventScroll: true });
  }, [live, autosave.loaded, meeting.id]);

  return (
    <div
      ref={rootRef}
      className="space-y-2"
      data-testid={live ? "live-notes-pad" : "my-notes"}
    >
      {meeting.source === "import" && (
        <p className="text-xs text-text/60">
          {t("meetings.notes.importedHint")}
        </p>
      )}
      {live && !compact && (
        <>
          <p className="text-xs text-mid-gray">{t("meetings.notes.padHint")}</p>
          <MeetingTemplatePicker meetingId={meeting.id} />
        </>
      )}
      {autosave.loaded ? (
        <div className="rounded-lg border border-mid-gray/20 px-3 py-3">
          <NoteBlocksEditor
            blocks={autosave.blocks}
            onChange={autosave.setBlocks}
            stampNewBlock={stamp}
            placeholder={t("meetings.notes.emptyHint")}
            onBlur={() => void autosave.flush()}
            autoFocus={live}
          />
        </div>
      ) : (
        <p className="py-3 text-center text-sm text-text/60">
          {t("meetings.list.loading")}
        </p>
      )}
      <NotesStatusLine autosave={autosave} />
    </div>
  );
};
