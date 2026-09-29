import React from "react";
import { useTranslation } from "react-i18next";
import type { Meeting } from "@/bindings";
import { NOTES_MAX_BYTES } from "@/lib/meetingNotes";
import { Alert } from "../../../ui/Alert";
import { NoteBlocksEditor } from "./NoteBlocksEditor";
import {
  recordingStamper,
  useNotesAutosave,
  type NotesAutosave,
} from "./useNotesAutosave";

/**
 * Speicherstatus, Groessenwarnung und Konflikthinweis eines Notizblocks;
 * gemeinsam fuer Aufnahmeseite (`LiveNotesPad`) und Detailansicht.
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
 * derselbe Autosave wie waehrend der Aufnahme. Laeuft die Aufnahme noch, tragen
 * neue Bloecke die Audioposition; nach dem Stopp und bei importierten
 * Besprechungen bleiben sie ohne Zeitstempel.
 */
export const MyNotesView: React.FC<{ meeting: Meeting }> = ({ meeting }) => {
  const { t } = useTranslation();
  const autosave = useNotesAutosave(meeting.id);
  const stamp = React.useMemo(
    () =>
      meeting.status === "recording" ? recordingStamper(meeting.id) : undefined,
    [meeting.id, meeting.status],
  );

  return (
    <div className="space-y-2" data-testid="my-notes">
      {meeting.source === "import" && (
        <p className="text-xs text-text/60">
          {t("meetings.notes.importedHint")}
        </p>
      )}
      {autosave.loaded ? (
        <div className="rounded-lg border border-mid-gray/20 px-3 py-3">
          <NoteBlocksEditor
            blocks={autosave.blocks}
            onChange={autosave.setBlocks}
            stampNewBlock={stamp}
            placeholder={t("meetings.notes.emptyHint")}
            onBlur={() => void autosave.flush()}
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
