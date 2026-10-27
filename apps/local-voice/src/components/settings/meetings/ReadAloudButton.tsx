import React, { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { FileAudio, Volume2, X } from "lucide-react";
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
  subscribeReadAloud,
  suspendReadingIfOwner,
} from "./readAloudSession";
import {
  ReadAloudControls,
  ReadAloudVoice,
  useReadAloudView,
} from "./ReadAloudBar";

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
 * gewählten Stimme. Ein Klick auf "Vorlesen" ersetzt den Knopf IN DERSELBEN Zeile durch
 * die Steuerung (`ReadAloudControls`: Satz zurück, Pause/Weiter, Satz vor, Stopp, Zähler,
 * Stimme); sie bleibt, solange eine Sitzung besteht -- auch pausiert, auch nach einem
 * Wechsel der Aufnahme oder des Reiters (der pausiert nur). Erst Stopp oder das Ende
 * blendet sie aus. "Audio speichern" steht immer dahinter.
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
  const view = useReadAloudView(speakKey);
  const speaking = session?.key === speakKey;
  const saving = exporting?.key === speakKey ? exporting : null;

  // Ohne den Reiter (Wechsel von Protokoll zu Transkript, andere Aufnahme) verstummt die
  // Stimme: sie PAUSIERT, die Sitzung bleibt -- zurück am selben Inhalt geht es mit Weiter
  // an derselben Stelle weiter.
  useEffect(() => () => suspendReadingIfOwner(speakKey), [speakKey]);

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
      {view ? (
        <ReadAloudControls speakKey={speakKey} testId={testId} />
      ) : (
        <IconAction
          size="sm"
          icon={Volume2}
          label={t("meetings.readAloud.speak")}
          description={t("meetings.readAloud.speakHint")}
          testId={testId}
          disabled={disabled}
          onClick={() => void onClick()}
        />
      )}
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
      {/* Die Stimme steht zuletzt und nimmt den Rest der Zeile. */}
      <ReadAloudVoice speakKey={speakKey} testId={testId} />
    </>
  );
};
