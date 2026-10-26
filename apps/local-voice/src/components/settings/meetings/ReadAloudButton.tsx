import React, { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Square, Volume2 } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { markdownToSpeech } from "@/lib/speechText";
import { IconAction } from "../../ui/IconAction";

/**
 * Wer gerade vorliest. Die Vorlese-Sitzung des Backends ist EINE für die ganze App
 * (Vorlesen-Seite, Aufnahmen): ein Knopf, der "Stopp" zeigt, darf deshalb nur der sein,
 * der sie gestartet hat. `token` unterscheidet zwei Läufe desselben Knopfes.
 */
interface Owner {
  key: string;
  token: number;
  stopped: boolean;
}

let owner: Owner | null = null;
let tokenCounter = 0;
const subscribers = new Set<() => void>();

const setOwner = (next: Owner | null) => {
  owner = next;
  subscribers.forEach((fn) => fn());
};
const subscribe = (fn: () => void) => {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
};
const ownerKey = () => owner?.key ?? null;

/** Wiedergabe beenden, wenn der Knopf `key` sie gestartet hat (Reiter-/Aufnahmewechsel). */
const stopIfOwner = (key: string) => {
  if (owner?.key !== key) return;
  owner.stopped = true;
  void commands.ttsCancel();
  setOwner(null);
};

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
}

/**
 * "Vorlesen" für erzeugte Inhalte der Aufnahmen (Protokoll, Dokumente, KI-Notizen,
 * Projektprotokoll). Keine eigene Sprachausgabe: derselbe Befehl `tts_speak_text` wie auf
 * der Vorlesen-Seite, mit der dort gewählten Stimme. Der Befehl kehrt erst nach der
 * Wiedergabe zurück; solange steht der Knopf auf "Stopp" (`tts_cancel`).
 */
export const ReadAloudButton: React.FC<ReadAloudButtonProps> = ({
  speakKey,
  getMarkdown,
  testId,
  disabled,
}) => {
  const { t } = useTranslation();
  const speaking = useSyncExternalStore(subscribe, ownerKey) === speakKey;

  // Ohne den Reiter (Wechsel von Protokoll zu Transkript, andere Aufnahme) verstummt die Stimme.
  useEffect(() => () => stopIfOwner(speakKey), [speakKey]);

  // Eine Kürzung an der Zeichengrenze muss man hören UND wissen.
  useEffect(() => {
    if (!speaking) return;
    const un = listen<{ limit: number; total: number }>(
      "tts-text-truncated",
      (e) => toast.info(t("meetings.readAloud.truncated", e.payload)),
    );
    return () => void un.then((off) => off());
  }, [speaking, t]);

  const start = async () => {
    const markdown = await getMarkdown();
    const text = markdown ? markdownToSpeech(markdown) : "";
    if (!text) {
      toast.info(t("meetings.readAloud.empty"));
      return;
    }
    const run: Owner = { key: speakKey, token: ++tokenCounter, stopped: false };
    setOwner(run);
    try {
      const result = await commands.ttsSpeakText(text);
      if (result.status === "error" && !run.stopped) {
        toast.error(t("meetings.readAloud.error", { error: result.error }));
      }
    } catch (e) {
      if (!run.stopped) {
        toast.error(t("meetings.readAloud.error", { error: String(e) }));
      }
    } finally {
      if (owner?.token === run.token) setOwner(null);
    }
  };

  const onClick = () => {
    if (speaking) {
      stopIfOwner(speakKey);
      return;
    }
    void start();
  };

  return (
    <IconAction
      size="sm"
      icon={speaking ? Square : Volume2}
      label={
        speaking ? t("meetings.readAloud.stop") : t("meetings.readAloud.speak")
      }
      description={
        speaking
          ? t("meetings.readAloud.stopHint")
          : t("meetings.readAloud.speakHint")
      }
      testId={testId}
      disabled={disabled && !speaking}
      aria-pressed={speaking}
      onClick={onClick}
    />
  );
};
