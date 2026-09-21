import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { commands, type PackagePreview, type PageInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { exportFileName } from "@/lib/utils/exportName";

export const PAGE_PACKAGE_EXT = "lvpage";

/**
 * Seite als Paket sichern: Arbeitsstand, Projektdateien und auf Wunsch die
 * verwendeten Stimmen. Stimmen sind Aufnahmen von Menschen -- wer sie
 * weitergibt, bestaetigt, dass er das darf (Urheber- und
 * Persoenlichkeitsrecht). Ohne Haken keine Stimmen im Paket.
 */
export const PageExportDialog: React.FC<{
  page: PageInfo | null;
  onClose: () => void;
}> = ({ page, onClose }) => {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<PackagePreview | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [rights, setRights] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!page) return;
    setPreview(null);
    setSelected(new Set());
    setRights(false);
    setError(null);
    void commands.pagesExportPreview(page.id).then((r) => {
      if (r.status === "ok") {
        setPreview(r.data);
        setSelected(
          new Set(r.data.voices.filter((v) => v.present).map((v) => v.id)),
        );
      } else {
        setError(r.error);
      }
    });
  }, [page]);

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const run = async () => {
    if (!page) return;
    setError(null);
    let target: string | null = null;
    try {
      target = await save({
        defaultPath: exportFileName(page.title, "Seite", PAGE_PACKAGE_EXT),
        filters: [
          {
            name: t("tts.pages.package.filter"),
            extensions: [PAGE_PACKAGE_EXT],
          },
        ],
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    if (typeof target !== "string") return;
    setBusy(true);
    const result = await commands.pagesExport(
      page.id,
      target,
      [...selected],
      rights,
    );
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    toast.success(t("tts.pages.package.exported", { path: result.data }));
    onClose();
  };

  const needsRights = selected.size > 0;
  return (
    <Dialog
      open={page !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t("tts.pages.package.exportTitle")}
      description={t("tts.pages.package.exportDescription")}
      closeLabel={t("common.close")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("tts.stopConfirmCancel")}
          </Button>
          <Button
            onClick={() => void run()}
            disabled={busy || !preview || (needsRights && !rights)}
            data-testid="page-export-run"
          >
            {busy
              ? t("tts.pages.package.exporting")
              : t("tts.pages.package.exportRun")}
          </Button>
        </>
      }
    >
      {preview ? (
        <div className="space-y-4 text-sm">
          <p>
            <span className="font-medium">{preview.title}</span>
            {" · "}
            {t("tts.pages.package.filesCount", { count: preview.files.length })}
          </p>
          {preview.voices.length > 0 ? (
            <div className="space-y-1">
              <span className="font-medium">
                {t("tts.pages.package.voicesUsed")}
              </span>
              <div className="rounded-md border border-mid-gray/20 divide-y divide-mid-gray/10">
                {preview.voices.map((v) => (
                  <label
                    key={v.id}
                    className={`flex items-center gap-2 px-2 py-1 ${
                      v.present
                        ? "cursor-pointer hover:bg-mid-gray/10"
                        : "opacity-50"
                    }`}
                  >
                    <input
                      type="checkbox"
                      checked={selected.has(v.id)}
                      disabled={!v.present}
                      onChange={() => toggle(v.id)}
                    />
                    <span className="truncate">{v.display_name}</span>
                    {!v.present && (
                      <span className="text-xs text-text/50">
                        {t("tts.pages.package.voiceIncomplete")}
                      </span>
                    )}
                  </label>
                ))}
              </div>
              <label
                className={`mt-2 flex items-start gap-2 rounded-md border px-2 py-2 ${
                  needsRights
                    ? "border-amber-500/40 bg-amber-500/5"
                    : "border-mid-gray/20 opacity-60"
                }`}
              >
                <input
                  type="checkbox"
                  checked={rights}
                  disabled={!needsRights}
                  onChange={(e) => setRights(e.target.checked)}
                  className="mt-0.5"
                  data-testid="page-export-rights"
                />
                <span className="text-xs">
                  <span className="font-medium">
                    {t("tts.pages.package.rightsTitle")}
                  </span>
                  <br />
                  {t("tts.pages.package.rightsBody")}
                </span>
              </label>
            </div>
          ) : (
            <p className="text-xs text-text/60">
              {t("tts.pages.package.noVoices")}
            </p>
          )}
          {error && <p className="text-red-400 break-words">{error}</p>}
        </div>
      ) : (
        <p className="text-sm text-text/60">
          {error ?? t("tts.pages.package.loading")}
        </p>
      )}
    </Dialog>
  );
};

/**
 * Paket einspielen: Vorschau (Titel, Dateien, Stimmen -- vorhanden oder
 * neu), dann als NEUE Seite anlegen; Stimmen nur auf Wunsch und nie eine
 * vorhandene ueberschreiben.
 */
export const PageImportDialog: React.FC<{
  open: boolean;
  onClose: () => void;
  onImported: (page: PageInfo) => void;
}> = ({ open: isOpen, onClose, onImported }) => {
  const { t } = useTranslation();
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<PackagePreview | null>(null);
  const [importVoices, setImportVoices] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    setPath(null);
    setPreview(null);
    setError(null);
    setImportVoices(true);
    void (async () => {
      try {
        const picked = await open({
          multiple: false,
          filters: [
            {
              name: t("tts.pages.package.filter"),
              extensions: [PAGE_PACKAGE_EXT],
            },
          ],
        });
        if (typeof picked !== "string") {
          onClose();
          return;
        }
        setPath(picked);
        const r = await commands.pagesPackageInspect(picked);
        if (r.status === "ok") setPreview(r.data);
        else setError(r.error);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    // onClose/t sind stabil genug; der Dialog oeffnet je Klick einmal.
  }, [isOpen]);

  const run = async () => {
    if (!path) return;
    setBusy(true);
    const result = await commands.pagesImport(path, importVoices);
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    toast.success(
      t("tts.pages.package.imported", { title: result.data.title }),
    );
    onImported(result.data);
    onClose();
  };

  const newVoices = preview?.voices.filter((v) => !v.present) ?? [];
  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t("tts.pages.package.importTitle")}
      closeLabel={t("common.close")}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("tts.stopConfirmCancel")}
          </Button>
          <Button
            onClick={() => void run()}
            disabled={busy || !preview}
            data-testid="page-import-run"
          >
            {busy
              ? t("tts.pages.package.importing")
              : t("tts.pages.package.importRun")}
          </Button>
        </>
      }
    >
      {preview ? (
        <div className="space-y-3 text-sm">
          <p>
            <span className="font-medium">{preview.title}</span>
            {" · "}
            {t("tts.pages.package.filesCount", { count: preview.files.length })}
          </p>
          {preview.voices.length > 0 && (
            <div className="space-y-1">
              <span className="font-medium">
                {t("tts.pages.package.voicesInPackage")}
              </span>
              <ul className="rounded-md border border-mid-gray/20 divide-y divide-mid-gray/10">
                {preview.voices.map((v) => (
                  <li key={v.id} className="flex items-center gap-2 px-2 py-1">
                    <span className="truncate">{v.display_name}</span>
                    <span className="text-xs text-text/50">
                      {v.present
                        ? t("tts.pages.package.voicePresent")
                        : t("tts.pages.package.voiceNew")}
                    </span>
                  </li>
                ))}
              </ul>
              {newVoices.length > 0 && (
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={importVoices}
                    onChange={(e) => setImportVoices(e.target.checked)}
                  />
                  {t("tts.pages.package.importVoices", {
                    count: newVoices.length,
                  })}
                </label>
              )}
              {!preview.rights_confirmed && newVoices.length > 0 && (
                <p className="text-xs text-amber-500">
                  {t("tts.pages.package.rightsMissing")}
                </p>
              )}
            </div>
          )}
          {error && <p className="text-red-400 break-words">{error}</p>}
        </div>
      ) : (
        <p className="text-sm text-text/60">
          {error ?? t("tts.pages.package.loading")}
        </p>
      )}
    </Dialog>
  );
};
