import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events } from "@/bindings";
import { SettingsGroup } from "../../../ui/SettingsGroup";
import { NoteBlocksEditor } from "./NoteBlocksEditor";
import { NotesStatusLine } from "./MyNotesView";
import { MeetingTemplatePicker } from "./TemplatePicker";
import { recordingStamper, useNotesAutosave } from "./useNotesAutosave";

/**
 * Notizblock neben dem Live-Transkript (M1, P1c). Erscheint mit dem Start einer
 * Aufnahme und bleibt danach stehen (wie das Transkript), damit nichts unter
 * den Fingern verschwindet; jeder neue Block merkt sich die Audioposition, solange
 * die Aufnahme laeuft. Gespeichert wird entprellt und sofort bei Blur, beim
 * Verdecken des Fensters und beim Ende der Aufnahme.
 */
export const LiveNotesPad: React.FC = () => {
  const { t } = useTranslation();
  const [meetingId, setMeetingId] = useState<string | null>(null);
  // Die Vorlagenwahl der laufenden Besprechung; davor waehlt die Aufnahmekarte
  // (RecorderCard), danach die Detailansicht - nie zwei Wahlen zugleich.
  const [recording, setRecording] = useState(false);
  const autosave = useNotesAutosave(meetingId);
  const { flush } = autosave;

  // Seite wurde waehrend einer laufenden Aufnahme geoeffnet: Besprechung erfragen.
  useEffect(() => {
    let cancelled = false;
    void commands.meetingsRecordingPosition().then((result) => {
      if (!cancelled && result.status === "ok" && result.data) {
        setMeetingId((prev) => prev ?? result.data!.meeting_id);
        setRecording(true);
      }
    });
    const un = events.meetingEvent.listen((e) => {
      const payload = e.payload;
      if (payload.kind !== "state") return;
      setRecording(
        payload.status === "recording" || payload.status === "paused",
      );
      if (payload.status === "recording") {
        setMeetingId(payload.meeting_id);
      } else if (payload.status === "processing") {
        // Aufnahme endet: Ungespeichertes sofort sichern.
        void flush();
      }
    });
    return () => {
      cancelled = true;
      un.then((f) => f());
    };
  }, [flush]);

  const stamp = useMemo(
    () => (meetingId ? recordingStamper(meetingId) : undefined),
    [meetingId],
  );

  if (!meetingId) return null;

  return (
    <SettingsGroup>
      <div className="space-y-2 px-4 py-3" data-testid="live-notes-pad">
        {/* Titel im Kasten (nicht darueber): so beginnen Notizblock und
            Transkript nebeneinander auf derselben Hoehe. */}
        <div>
          <h2 className="text-xs font-medium text-mid-gray uppercase tracking-wide">
            {t("meetings.notes.padTitle")}
          </h2>
          <p className="text-xs text-mid-gray mt-1">
            {t("meetings.notes.padHint")}
          </p>
        </div>
        {recording && <MeetingTemplatePicker meetingId={meetingId} />}
        <div className="max-h-80 min-h-[8rem] overflow-y-auto">
          {autosave.loaded && (
            <NoteBlocksEditor
              blocks={autosave.blocks}
              onChange={autosave.setBlocks}
              stampNewBlock={stamp}
              placeholder={t("meetings.notes.emptyHint")}
              onBlur={() => void flush()}
            />
          )}
        </div>
        <NotesStatusLine autosave={autosave} />
      </div>
    </SettingsGroup>
  );
};
