import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import { commands, type Folder, type Meeting } from "@/bindings";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Alert } from "../../../ui/Alert";
import { folderErrorText } from "./FolderChips";

interface FolderPickerDialogProps {
  /** `null` = geschlossen. */
  meeting: Meeting | null;
  folders: Folder[];
  onClose: () => void;
  /** Nach dem Speichern (oder einem neu angelegten Ordner): neu laden. */
  onSaved: () => void;
}

/** „In Ordner …“: Mehrfachwahl (n:m). Gespeichert wird genau die Auswahl
 *  (`meetings_set_folders`), vorbelegt mit den aktuellen Ordnern. */
export const FolderPickerDialog: React.FC<FolderPickerDialogProps> = ({
  meeting,
  folders,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const meetingId = meeting?.id ?? null;

  useEffect(() => {
    if (!meetingId) return;
    let alive = true;
    setLoaded(false);
    setError(null);
    setNewName("");
    void commands.meetingsGetFolders(meetingId).then((result) => {
      if (!alive) return;
      if (result.status === "ok") {
        setSelected(result.data ?? []);
      } else {
        setSelected([]);
        setError(folderErrorText(result.error, t));
      }
      setLoaded(true);
    });
    return () => {
      alive = false;
    };
  }, [meetingId, t]);

  const toggle = (id: string) =>
    setSelected((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id],
    );

  const createFolder = async () => {
    if (busy || newName.trim() === "") return;
    setBusy(true);
    const result = await commands.meetingFoldersSave(null, newName, null);
    setBusy(false);
    if (result.status === "ok" && result.data) {
      const created = result.data;
      setSelected((prev) => [...prev, created.id]);
      setNewName("");
      setError(null);
      onSaved();
    } else {
      setError(
        folderErrorText(result.status === "error" ? result.error : "", t),
      );
    }
  };

  const save = async () => {
    if (!meetingId || busy) return;
    setBusy(true);
    // Nur Ordner, die es noch gibt (ein parallel geloeschter faellt heraus).
    const known = selected.filter((id) => folders.some((f) => f.id === id));
    const result = await commands.meetingsSetFolders(meetingId, known);
    setBusy(false);
    if (result.status === "error") {
      setError(folderErrorText(result.error, t));
      return;
    }
    onSaved();
    onClose();
  };

  return (
    <Dialog
      open={meeting !== null}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title={t("meetings.folders.pickerTitle")}
      description={meeting?.title}
      closeLabel={t("meetings.folders.cancel")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("meetings.folders.cancel")}
          </Button>
          <Button onClick={save} disabled={busy || !loaded}>
            {t("meetings.folders.save")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <p className="text-xs text-text/60">
          {t("meetings.folders.pickerHint")}
        </p>
        {folders.length === 0 ? (
          <p className="text-sm text-text/60">
            {t("meetings.folders.pickerEmpty")}
          </p>
        ) : (
          <ul className="space-y-1" role="group">
            {folders.map((folder) => {
              const checked = selected.includes(folder.id);
              return (
                <li key={folder.id}>
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={checked}
                    disabled={!loaded}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-start text-sm hover:bg-mid-gray/10 cursor-pointer disabled:cursor-default"
                    onClick={() => toggle(folder.id)}
                  >
                    <span
                      className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${
                        checked
                          ? "border-logo-primary bg-logo-primary text-background"
                          : "border-mid-gray/60"
                      }`}
                      aria-hidden="true"
                    >
                      {checked && <Check width={12} height={12} />}
                    </span>
                    <span className="truncate">{folder.name}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void createFolder();
          }}
        >
          <Input
            value={newName}
            maxLength={60}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("meetings.folders.namePlaceholder")}
            aria-label={t("meetings.folders.newTitle")}
            variant="compact"
            className="flex-1 font-normal"
          />
          <Button
            type="submit"
            variant="secondary"
            size="sm"
            disabled={busy || newName.trim() === ""}
          >
            {t("meetings.folders.create")}
          </Button>
        </form>
        {error && <Alert variant="error">{error}</Alert>}
      </div>
    </Dialog>
  );
};
