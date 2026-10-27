import React, { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { FileAudio, Square, Volume2, X } from "lucide-react";
import { toast } from "sonner";
import { markdownToSpeech } from "@/lib/speechText";
import { exportFileName } from "@/lib/utils/exportName";
import { useSettings } from "../../../hooks/useSettings";
import { IconAction } from "../../ui/IconAction";
import {
  cancelExport,
  getReadAloudExport,
  getReadAloudSession,
  startExport,
  startReading,
  stopReading,
  stopReadingIfOwner,
  subscribeReadAloud,
} from "./readAloudSession";

interface ReadAloudButtonProps {
  /** Wer vorliest, z. B. `minutes:<Besprechung>:<Dokument>`; je Inhalt verschieden. */
  speakKey: string;
  /**
   * Der Markdown-Text, der vorgelesen wird. Darf erst beim Klick entstehen (Notizen);
   * `null` = nichts zu lesen.
   */
  getMarkdown: () => string | null | Promise<string | null>;
  testId: string;
  disabled?: boolean;
  /** Namensstamm der Audiodatei (z. B. Titel der Aufnahme); mit Zeitstempel. */
  fileStem?: string;
}

/**
 * "Vorlesen" und "Audio speichern" für erzeugte Inhalte der Aufnahmen (Protokoll,
 * Dokumente, KI-Notizen, Projektprotokoll). Keine eigene Sprachausgabe: dieselben Befehle
 * wie auf der Vorlesen-Seite (`tts_speak_text`, `tts_speak_to_file`), mit der dort
 * gewählten Stimme. Solange vorgelesen wird, steht der Knopf auf "Beenden" und
 * `ReadAloudBar` zeigt Pause/Weiter, Satzsprung und Stimmenwahl.
 */
export const ReadAloudButton: React.FC<ReadAloudButtonProps> = ({
  speakKey,
  getMarkdown,
  testId,
  disabled,
  fileStem,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const session = useSyncExternalStore(subscribeReadAloud, getReadAloudSession);
  const exporting = useSyncExternalStore(
    subscribeReadAloud,
    getReadAloudExport,
  );
  const speaking = session?.key === speakKey;
  const saving = exporting?.key === speakKey ? exporting : null;

  // Ohne den Reiter (Wechsel von Protokoll zu Transkript, andere Aufnahme) verstummt die Stimme.
  useEffect(() => () => stopReadingIfOwner(speakKey), [speakKey]);

  // Eine Kürzung an der Zeichengrenze muss man hören UND wissen.
  useEffect(() => {
    if (!speaking) return;
    const un = listen<{ limit: number; total: number }>(
      "tts-text-truncated",
      (e) => toast.info(t("meetings.readAloud.truncated", e.payload)),
    );
    return () => void un.then((off) => off());
  }, [speaking, t]);

  const spokenText = async () => {
    const markdown = await getMarkdown();
    const text = markdown ? markdownToSpeech(markdown) : "";
    if (!text) toast.info(t("meetings.readAloud.empty"));
    return text;
  };

  const onClick = async () => {
    if (speaking) {
      stopReading();
      return;
    }
    const text = await spokenText();
    if (text) startReading(speakKey, text);
  };

  const onSave = async () => {
    if (saving) {
      cancelExport();
      return;
    }
    const text = await spokenText();
    if (!text) return;
    // Das Format steht in den Einstellungen -- der Dialog fragt es nicht noch einmal.
    // "opus" schlägt bewusst eine .wav vor: einen Opus-Kodierer gibt es nicht, der Export
    // schreibt dafür WAV-Daten.
    const format = (getSetting("tts_export_format") ?? "wav").toLowerCase();
    const ext = format === "mp3" ? "mp3" : "wav";
    const target = await save({
      filters: [{ name: ext === "mp3" ? "MP3" : "WAV", extensions: [ext] }],
      defaultPath: exportFileName(
        fileStem,
        t("meetings.readAloud.fileFallback"),
        ext,
      ),
    });
    if (typeof target !== "string") return;
    await startExport(speakKey, text, target);
  };

  return (
    <>
      <IconAction
        size="sm"
        icon={speaking ? Square : Volume2}
        label={
          speaking
            ? t("meetings.readAloud.stop")
            : t("meetings.readAloud.speak")
        }
        description={
          speaking
            ? t("meetings.readAloud.stopHint")
            : t("meetings.readAloud.speakHint")
        }
        testId={testId}
        disabled={disabled && !speaking}
        aria-pressed={speaking}
        onClick={() => void onClick()}
      />
      <IconAction
        size="sm"
        icon={saving ? X : FileAudio}
        label={
          saving
            ? saving.total > 0
              ? t("meetings.readAloud.saveCancelProgress", {
                  position: saving.position,
                  total: saving.total,
                })
              : t("meetings.readAloud.saveCancel")
            : t("meetings.readAloud.save")
        }
        description={
          saving
            ? t("meetings.readAloud.saveCancelHint")
            : t("meetings.readAloud.saveHint")
        }
        testId={`${testId}-save`}
        disabled={disabled && !saving}
        aria-busy={saving !== null}
        onClick={() => void onSave()}
      />
    </>
  );
};
