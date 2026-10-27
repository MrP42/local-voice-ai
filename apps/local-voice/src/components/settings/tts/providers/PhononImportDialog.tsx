import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type OwnVoice } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import {
  NAME_MAX,
  asPhononError,
  fileBaseName,
  nameFromFile,
} from "./phononVoicesModel";

interface Props {
  /** Gewählte `.safetensors`-Datei; `null` = Dialog geschlossen. */
  path: string | null;
  onClose: () => void;
  onDone: (voice: OwnVoice) => void;
}

/**
 * Eigene Stimme importieren: Name vergeben, prüfen, ablegen. Kein Netzzugriff.
 * Gibt es den Namen schon, fragt der Dialog zurück, bevor er ersetzt.
 */
export const PhononImportDialog: React.FC<Props> = ({
  path,
  onClose,
  onDone,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askOverwrite, setAskOverwrite] = useState(false);

  useEffect(() => {
    if (path === null) return;
    setName(nameFromFile(path));
    setError(null);
    setAskOverwrite(false);
    setBusy(false);
  }, [path]);

  const run = async (overwrite: boolean) => {
    if (path === null) return;
    setBusy(true);
    setError(null);
    const result = await commands
      .phononOwnVoiceImport(path, name.trim(), overwrite)
      .catch((e) => ({ status: "error" as const, error: asPhononError(e) }));
    setBusy(false);
    if (result.status === "ok") {
      onDone(result.data);
      return;
    }
    if (result.error.kind === "exists") {
      setAskOverwrite(true);
      return;
    }
    setAskOverwrite(false);
    setError(result.error.message);
  };

  const canRun = name.trim() !== "" && name.trim().length <= NAME_MAX && !busy;

  return (
    <Dialog
      open={path !== null}
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
      title={t("tts.providers.phononVoices.import.title")}
      description={t("tts.providers.phononVoices.import.description")}
      closeLabel={t("common.close")}
      dismissible={!busy}
      className="max-w-lg"
      footer={
        <>
          <Button
            type="button"
            variant="primary"
            disabled={!canRun}
            data-testid="phonon-import-run"
            onClick={() => void run(askOverwrite)}
          >
            {askOverwrite
              ? t("tts.providers.phononVoices.import.replace")
              : t("tts.providers.phononVoices.import.run")}
          </Button>
          <Button
            type="button"
            variant="secondary"
            disabled={busy}
            onClick={onClose}
          >
            {t("common.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3 text-sm" data-testid="phonon-import-dialog">
        <p className="text-xs text-text/70" data-testid="phonon-import-file">
          {t("tts.providers.phononVoices.import.file", {
            file: path ? fileBaseName(path) : "",
          })}
        </p>
        <div className="space-y-1">
          <label
            htmlFor="phonon-import-name"
            className="block text-xs font-medium text-text/80"
          >
            {t("tts.providers.phononVoices.import.name")}
          </label>
          <Input
            id="phonon-import-name"
            type="text"
            value={name}
            maxLength={NAME_MAX}
            autoComplete="off"
            className="w-full"
            data-testid="phonon-import-name"
            onChange={(e) => {
              setName(e.target.value);
              setAskOverwrite(false);
              setError(null);
            }}
          />
        </div>
        <p className="text-xs text-text/70">
          {t("tts.providers.phononVoices.import.hint")}
        </p>
        {askOverwrite && (
          <p
            role="alert"
            className="text-xs text-yellow-500"
            data-testid="phonon-import-overwrite"
          >
            {t("tts.providers.phononVoices.import.overwrite", {
              name: name.trim(),
            })}
          </p>
        )}
        {error && (
          <p
            role="alert"
            className="text-xs text-red-500"
            data-testid="phonon-import-error"
          >
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
