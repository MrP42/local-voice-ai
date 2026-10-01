import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type Direction, type IntegrationView } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { DIRECTIONS, errorText } from "./model";

interface FolderDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (view: IntegrationView) => void;
}

/** Letzter Ordnername eines Pfads, als Vorschlag fuer den Namen. */
const nameFromPath = (path: string): string => {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
};

/**
 * Ordner einrichten: Name, Pfad und Richtung. Das Backend prueft den Pfad
 * (vollstaendig, vorhanden, ein Ordner) VOR dem Anlegen; ein Fehler steht im
 * Dialog und es entsteht nichts. Die Sandbox gegen `..` und Verknuepfungen
 * gehoert dem Ordner-Adapter (A6).
 */
export const FolderDialog: React.FC<FolderDialogProps> = ({
  open: isOpen,
  onOpenChange,
  onCreated,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [direction, setDirection] = useState<Direction>("both");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nameTouched, setNameTouched] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    setName("");
    setPath("");
    setDirection("both");
    setBusy(false);
    setError(null);
    setNameTouched(false);
  }, [isOpen]);

  const pick = async () => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) {
        setPath(picked);
        setError(null);
        if (!nameTouched) setName(nameFromPath(picked));
      }
    } catch {
      /* Abbruch des Dialogs */
    }
  };

  const explain = (raw: string) => {
    const text = t(`integrations.errors.${raw}`, { defaultValue: "" });
    return text || raw || t("integrations.errors.generic");
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await commands.integrationCreate(
        "folder",
        name.trim(),
        direction,
        path.trim(),
      );
      if (result.status === "ok") {
        onCreated(result.data);
        onOpenChange(false);
        return;
      }
      setError(explain(errorText(result.error)));
    } catch (e) {
      setError(explain(errorText(e)));
    }
    setBusy(false);
  };

  const ready = name.trim() !== "" && path.trim() !== "";

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("integrations.folder.title")}
      description={t("integrations.folder.description")}
      closeLabel={t("integrations.close")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid="folder-cancel"
          >
            {t("integrations.cancel")}
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={!ready || busy}
            data-testid="folder-submit"
          >
            {t("integrations.folder.create")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="folder-dialog">
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="folder-name">
            {t("integrations.folder.name")}
          </label>
          <Input
            id="folder-name"
            className="w-full"
            value={name}
            maxLength={120}
            onChange={(e) => {
              setName(e.target.value);
              setNameTouched(true);
            }}
            data-testid="folder-name"
          />
        </div>
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="folder-path">
            {t("integrations.folder.path")}
          </label>
          <div className="flex flex-wrap gap-2">
            <Input
              id="folder-path"
              className="min-w-0 flex-1 basis-48"
              value={path}
              placeholder={t("integrations.folder.pathPlaceholder")}
              onChange={(e) => {
                setPath(e.target.value);
                setError(null);
              }}
              data-testid="folder-path"
            />
            <Button
              variant="secondary"
              onClick={() => void pick()}
              data-testid="folder-pick"
            >
              {t("integrations.folder.pick")}
            </Button>
          </div>
        </div>
        <div className="space-y-1">
          <div className="text-sm font-medium" id="folder-direction">
            {t("integrations.detail.direction")}
          </div>
          <div
            role="radiogroup"
            aria-labelledby="folder-direction"
            className="inline-flex max-w-full flex-wrap overflow-hidden rounded-lg border border-mid-gray/30"
          >
            {DIRECTIONS.map((d) => (
              <button
                key={d}
                type="button"
                role="radio"
                aria-checked={direction === d}
                onClick={() => setDirection(d)}
                data-testid={`folder-direction-${d}`}
                className={`min-h-9 cursor-pointer px-3 py-1 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                  direction === d
                    ? "bg-logo-primary text-on-accent"
                    : "text-text hover:bg-mid-gray/15"
                }`}
              >
                {t(`integrations.directions.${d}`)}
              </button>
            ))}
          </div>
          <p className="text-xs text-text-muted">
            {t("integrations.folder.directionHint")}
          </p>
        </div>
        {error && (
          <p
            className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red"
            role="alert"
            data-testid="folder-error"
          >
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
