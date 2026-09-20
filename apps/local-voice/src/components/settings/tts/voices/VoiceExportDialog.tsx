import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { downloadDir } from "@tauri-apps/api/path";
import { FolderOpen } from "lucide-react";
import { commands, type VoiceInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";

interface VoiceExportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  voices: VoiceInfo[];
}

const LAST_DIR_KEY = "lv-voice-export-dir";

/** `stimmen-2026-09-20_1612` — Datum und Uhrzeit, damit zwei Sicherungen
 * am selben Tag nicht kollidieren. */
const defaultBaseName = (prefix: string, at = new Date()) => {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${prefix}-${at.getFullYear()}-${p(at.getMonth() + 1)}-${p(at.getDate())}_${p(at.getHours())}${p(at.getMinutes())}`;
};

/**
 * Sammel-Export: welche Stimmen, wohin, unter welchem Namen, gepackt oder
 * als Ordner. Alle Stimmen sind vorausgewaehlt — der haeufigste Fall ist
 * „alles sichern", das Abwaehlen ist die Ausnahme.
 */
export const VoiceExportDialog = ({
  open: isOpen,
  onOpenChange,
  voices,
}: VoiceExportDialogProps) => {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [dir, setDir] = useState("");
  const [baseName, setBaseName] = useState("");
  const [packed, setPacked] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{
    path: string;
    exported: number;
    failed: [string, string][];
  } | null>(null);

  // Beim Oeffnen frisch: alle Stimmen, letzter Ordner, Name mit Zeitstempel.
  useEffect(() => {
    if (!isOpen) return;
    setSelected(new Set(voices.map((v) => v.id)));
    setBaseName(defaultBaseName(t("tts.voices.exportAll.filePrefix")));
    setPacked(true);
    setError(null);
    setDone(null);
    let remembered = "";
    try {
      remembered = window.localStorage.getItem(LAST_DIR_KEY) ?? "";
    } catch {
      // kein localStorage — dann Downloads
    }
    if (remembered) {
      setDir(remembered);
    } else {
      void downloadDir()
        .then((d) => setDir(d))
        .catch(() => setDir(""));
    }
  }, [isOpen, voices, t]);

  const allSelected = selected.size === voices.length && voices.length > 0;
  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const pickDir = async () => {
    try {
      const picked = await open({
        directory: true,
        defaultPath: dir || undefined,
      });
      if (typeof picked === "string") setDir(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const targetPreview = useMemo(() => {
    if (!dir || !baseName.trim()) return "";
    const sep = dir.includes("/") && !dir.includes("\\") ? "/" : "\\";
    return `${dir.replace(/[\\/]+$/, "")}${sep}${baseName.trim()}${packed ? ".zip" : sep}`;
  }, [dir, baseName, packed]);

  const run = async () => {
    setError(null);
    setBusy(true);
    const result = await commands.ttsExportVoices(
      [...selected],
      dir,
      baseName.trim(),
      packed,
    );
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    try {
      window.localStorage.setItem(LAST_DIR_KEY, dir);
    } catch {
      // egal
    }
    setDone({
      path: result.data.path,
      exported: result.data.exported.length,
      failed: result.data.failed,
    });
  };

  const canRun =
    selected.size > 0 && dir.trim() !== "" && baseName.trim() !== "" && !busy;

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("tts.voices.exportAll.title")}
      description={t("tts.voices.exportAll.description")}
      closeLabel={t("common.close")}
      footer={
        done ? (
          <Button onClick={() => onOpenChange(false)}>
            {t("common.close")}
          </Button>
        ) : (
          <>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              {t("tts.stopConfirmCancel")}
            </Button>
            <Button onClick={() => void run()} disabled={!canRun}>
              {busy
                ? t("tts.voices.exportAll.running")
                : t("tts.voices.exportAll.run", { count: selected.size })}
            </Button>
          </>
        )
      }
    >
      {done ? (
        <div className="space-y-2 text-sm">
          <p>{t("tts.voices.exportAll.done", { count: done.exported })}</p>
          <p className="font-mono text-xs break-all text-text/70">
            {done.path}
          </p>
          {done.failed.length > 0 && (
            <div className="text-red-400">
              <p>
                {t("tts.voices.exportAll.failed", {
                  count: done.failed.length,
                })}
              </p>
              <ul className="list-disc pl-5 text-xs">
                {done.failed.map(([id, why]) => (
                  <li key={id}>
                    {id}: {why}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-4 text-sm">
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <span className="font-medium">
                {t("tts.voices.exportAll.which", {
                  selected: selected.size,
                  total: voices.length,
                })}
              </span>
              <button
                type="button"
                className="text-xs text-logo-primary hover:underline"
                onClick={() =>
                  setSelected(
                    allSelected ? new Set() : new Set(voices.map((v) => v.id)),
                  )
                }
              >
                {allSelected
                  ? t("tts.voices.exportAll.none")
                  : t("tts.voices.exportAll.all")}
              </button>
            </div>
            <div className="max-h-48 overflow-y-auto rounded-md border border-mid-gray/20 divide-y divide-mid-gray/10">
              {voices.map(({ id, meta }) => (
                <label
                  key={id}
                  className="flex items-center gap-2 px-2 py-1 cursor-pointer hover:bg-mid-gray/10"
                >
                  <input
                    type="checkbox"
                    checked={selected.has(id)}
                    onChange={() => toggle(id)}
                  />
                  <span className="truncate">{meta.display_name || id}</span>
                </label>
              ))}
            </div>
          </div>

          <div className="space-y-1">
            <span className="font-medium">
              {t("tts.voices.exportAll.where")}
            </span>
            <div className="flex gap-2">
              <Input
                type="text"
                value={dir}
                onChange={(e) => setDir(e.target.value)}
                className="flex-1 font-mono text-xs"
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void pickDir()}
              >
                <FolderOpen width={14} height={14} />
                {t("tts.voices.exportAll.choose")}
              </Button>
            </div>
          </div>

          <div className="space-y-1">
            <span className="font-medium">
              {t("tts.voices.exportAll.name")}
            </span>
            <Input
              type="text"
              value={baseName}
              onChange={(e) => setBaseName(e.target.value)}
              className="w-full"
            />
          </div>

          <label className="flex items-start gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={packed}
              onChange={(e) => setPacked(e.target.checked)}
              className="mt-0.5"
            />
            <span>
              <span className="font-medium">
                {t("tts.voices.exportAll.packed")}
              </span>
              <br />
              <span className="text-xs text-text/60">
                {packed
                  ? t("tts.voices.exportAll.packedHint")
                  : t("tts.voices.exportAll.folderHint")}
              </span>
            </span>
          </label>

          {targetPreview && (
            <p className="text-xs text-text/60 font-mono break-all">
              {targetPreview}
            </p>
          )}
          {error && <p className="text-red-400 break-words">{error}</p>}
        </div>
      )}
    </Dialog>
  );
};
