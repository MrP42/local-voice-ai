import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { commands, type Meeting } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { Alert } from "../../ui/Alert";
import { translateMeetingError } from "./meetingErrors";

interface RetranscribeControlProps {
  meeting: Meeting;
  /** Called once the run finished, so the caller can reload its segments. */
  onFinished: () => void;
  /** P8a: die Besprechung wird gerade verarbeitet: nicht noch einmal starten. */
  busy?: boolean;
}

/**
 * Re-runs a finished meeting's transcription, optionally with a different
 * model — the whole point of the control, since the model that was loaded
 * while recording is not necessarily the one that transcribes it best.
 *
 * Only offered when the meeting still has audio on disk: without it there is
 * nothing to transcribe again, and the existing transcript is the only copy
 * (retention policy, `MeetingRetentionSetting`).
 */
export const RetranscribeControl: React.FC<RetranscribeControlProps> = ({
  meeting,
  onFinished,
  busy = false,
}) => {
  const { t } = useTranslation();
  const { models, loadModels } = useModelStore();
  const [modelId, setModelId] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (models.length === 0) void loadModels();
  }, [models.length, loadModels]);

  const hasAudio = Boolean(meeting.mic_audio_path || meeting.system_audio_path);
  if (!hasAudio) return null;

  const options = [
    { value: "", label: t("meetings.retranscribe.configuredModel") },
    ...models
      .filter((model) => model.is_downloaded)
      .map((model) => ({ value: model.id, label: model.name })),
  ];

  const run = async () => {
    setError(null);
    setRunning(true);
    const result = await commands.meetingsRetranscribe(
      meeting.id,
      modelId === "" ? null : modelId,
    );
    setRunning(false);
    if (result.status === "error") {
      setError(translateMeetingError(result.error, t));
      return;
    }
    onFinished();
  };

  return (
    <div className="space-y-2 border border-mid-gray/20 rounded-md px-3 py-2">
      <p className="text-sm text-text/70">
        {t("meetings.retranscribe.description")}
      </p>
      {/* Wraps rather than squeezing: on a narrow window the dropdown gets the
          full row and the button drops beneath it. */}
      <div className="flex flex-wrap items-center gap-2">
        <Dropdown
          options={options}
          selectedValue={modelId}
          onSelect={setModelId}
          disabled={running || busy}
          className="flex-1 min-w-[12rem]"
        />
        <Button
          variant="secondary"
          size="sm"
          onClick={run}
          disabled={running || busy}
          className="shrink-0"
        >
          <RefreshCw
            width={14}
            height={14}
            className={running ? "animate-spin" : ""}
          />
          {running
            ? t("meetings.retranscribe.running")
            : t("meetings.retranscribe.start")}
        </Button>
      </div>
      {running && (
        <p className="text-xs text-text/60">
          {t("meetings.retranscribe.runningHint")}
        </p>
      )}
      {error && <Alert variant="error">{error}</Alert>}
    </div>
  );
};
