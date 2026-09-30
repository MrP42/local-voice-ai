import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Upload } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { IconAction } from "../../ui/IconAction";
import { translateMeetingError } from "./meetingErrors";

/** Dieselben Endungen wie beim Ziehen in die Liste (gleiche Import-Pipeline). */
const IMPORT_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "mp4",
  "mkv",
  "mov",
  "flac",
  "ogg",
  "vtt",
  "srt",
];

const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/**
 * Symbol "Datei importieren" der Bedienspalte. Wie beim Aufnehmen gilt: erst
 * die Einwilligung (Dass die Datei existiert, belegt keine), dann der Import.
 */
export const MeetingImportAction: React.FC = () => {
  const { t } = useTranslation();
  const [paths, setPaths] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);

  const pick = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Media", extensions: IMPORT_EXTENSIONS }],
    });
    if (typeof picked === "string") setPaths([picked]);
  };

  const confirm = async () => {
    const list = paths;
    if (!list || list.length === 0) return;
    setPaths(null);
    setBusy(true);
    for (const path of list) {
      const result = await commands.meetingsImportFile(path, true);
      if (result.status === "error") {
        toast.error(translateMeetingError(result.error, t));
      }
      notifyMeetingsChanged();
    }
    setBusy(false);
  };

  return (
    <>
      <IconAction
        icon={Upload}
        label={t("meetings.importAction.name")}
        description={t("meetings.importAction.hint")}
        testId="import-open"
        disabled={busy}
        onClick={() => void pick()}
      />
      <Dialog
        open={paths !== null}
        onOpenChange={(o) => {
          if (!o) setPaths(null);
        }}
        title={t("meetings.consent.title")}
        closeLabel={t("meetings.consent.cancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setPaths(null)}>
              {t("meetings.consent.cancel")}
            </Button>
            <Button onClick={() => void confirm()} disabled={busy}>
              {t("meetings.consent.confirm")}
            </Button>
          </>
        }
      >
        <p className="whitespace-pre-wrap text-sm text-text/80">
          {t("meetings.consent.importBody")}
        </p>
        {paths && (
          <ul className="mt-2 space-y-0.5 text-xs text-text/60">
            {paths.map((p) => (
              <li key={p} className="truncate">
                {baseName(p)}
              </li>
            ))}
          </ul>
        )}
      </Dialog>
    </>
  );
};
