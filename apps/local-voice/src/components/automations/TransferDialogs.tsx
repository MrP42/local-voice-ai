import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Check, Copy, FolderOpen, Save } from "lucide-react";
import {
  commands,
  type WorkflowIssue,
  type WorkflowItem,
  type WorkflowTemplate,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Textarea } from "../ui/Textarea";
import { errorOf } from "./useAutomations";

// ---------------------------------------------------------------------------
// Vorlagen
// ---------------------------------------------------------------------------

interface TemplateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  templates: WorkflowTemplate[];
  onPick: (template: WorkflowTemplate | null) => void;
}

/** „Neuer Ablauf“: aus einer mitgelieferten Vorlage oder leer. Die Vorlage wird erst im
 *  Editor angepasst (Ordner, Kalender, Konten waehlen) und dann gespeichert. */
export const TemplateDialog: React.FC<TemplateDialogProps> = ({
  open: isOpen,
  onOpenChange,
  templates,
  onPick,
}) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("automations.templates.title")}
      description={t("automations.templates.description")}
      closeLabel={t("automations.close")}
      className="max-w-2xl"
    >
      <ul className="space-y-3" data-testid="template-list">
        {templates.map((tpl) => (
          <li
            key={tpl.id}
            className="space-y-1 rounded-lg border border-mid-gray/30 p-3"
            data-testid="template-item"
            data-template-id={tpl.id}
          >
            <p className="break-words text-sm font-semibold">
              {t(`automations.templates.names.${tpl.id}`, {
                defaultValue: tpl.name,
              })}
            </p>
            <p className="text-xs text-text-muted">{tpl.description}</p>
            <Button
              size="sm"
              onClick={() => onPick(tpl)}
              data-testid="template-use"
            >
              {t("automations.templates.use")}
            </Button>
          </li>
        ))}
        <li className="space-y-1 rounded-lg border border-dashed border-mid-gray/40 p-3">
          <p className="text-sm font-semibold">
            {t("automations.templates.blank")}
          </p>
          <p className="text-xs text-text-muted">
            {t("automations.templates.blankHint")}
          </p>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onPick(null)}
            data-testid="template-blank"
          >
            {t("automations.templates.useBlank")}
          </Button>
        </li>
      </ul>
    </Dialog>
  );
};

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

interface ImportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImported: (item: WorkflowItem) => void;
}

/** JSON einfuegen oder aus einer Datei lesen und als NEUEN Ablauf anlegen. Dieselbe Pruefung
 *  wie beim Speichern; der Import ist ausgeschaltet und im Trockenlauf. */
export const ImportDialog: React.FC<ImportDialogProps> = ({
  open: isOpen,
  onOpenChange,
  onImported,
}) => {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [issues, setIssues] = useState<WorkflowIssue[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setText("");
      setIssues([]);
      setError(null);
    }
  }, [isOpen]);

  const pickFile = async () => {
    setError(null);
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const r = await commands.workflowReadFile(path);
      if (r.status === "ok") {
        setText(r.data);
        setIssues([]);
      } else {
        setError(r.error);
      }
    } catch (e) {
      setError(errorOf(e) || t("automations.errors.generic"));
    }
  };

  const run = async () => {
    setBusy(true);
    setError(null);
    setIssues([]);
    try {
      const r = await commands.workflowImport(text);
      if (r.status === "error") setError(r.error);
      else if (r.data.workflow) {
        onImported(r.data.workflow);
        onOpenChange(false);
      } else {
        setIssues(r.data.issues);
      }
    } catch (e) {
      setError(errorOf(e) || t("automations.errors.generic"));
    }
    setBusy(false);
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("automations.import.title")}
      description={t("automations.import.description")}
      closeLabel={t("automations.close")}
      className="max-w-2xl"
      footer={
        <>
          <Button
            size="sm"
            disabled={busy || !text.trim()}
            onClick={() => void run()}
            data-testid="import-submit"
          >
            {t("automations.import.submit")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onOpenChange(false)}
          >
            {t("automations.cancel")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="import-dialog">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => void pickFile()}
          data-testid="import-file"
        >
          <FolderOpen size={14} aria-hidden="true" />
          {t("automations.import.chooseFile")}
        </Button>
        <div className="space-y-1">
          <label htmlFor="import-text" className="text-sm font-medium">
            {t("automations.import.text")}
          </label>
          <Textarea
            id="import-text"
            className="w-full font-mono"
            rows={10}
            value={text}
            spellCheck={false}
            onChange={(e) => setText(e.target.value)}
            data-testid="import-text"
          />
        </div>
        {error && (
          <p
            className="text-sm text-status-red"
            role="alert"
            data-testid="import-error"
          >
            {error}
          </p>
        )}
        {issues.length > 0 && (
          <div role="alert" data-testid="import-issues" className="space-y-1">
            <p className="text-sm font-medium text-status-red">
              {t("automations.import.invalid", { count: issues.length })}
            </p>
            <ul className="max-h-40 space-y-0.5 overflow-auto text-xs">
              {issues.map((i) => (
                <li key={`${i.path}:${i.message}`}>
                  <code className="me-1 rounded bg-mid-gray/20 px-1">
                    {i.path || "/"}
                  </code>
                  {i.message}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </Dialog>
  );
};

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

interface ExportDialogProps {
  item: WorkflowItem | null;
  onClose: () => void;
}

/** Die Definition als JSON: zum Kopieren oder als Datei. Sie enthaelt keine Geheimnisse
 *  (Konten sind nur Kennungen aus dem Register). */
export const ExportDialog: React.FC<ExportDialogProps> = ({
  item,
  onClose,
}) => {
  const { t } = useTranslation();
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<"idle" | "ok" | "failed">("idle");
  const [savedTo, setSavedTo] = useState<string | null>(null);

  const id = item?.id ?? null;
  useEffect(() => {
    setText(null);
    setError(null);
    setCopied("idle");
    setSavedTo(null);
    if (!id) return;
    let alive = true;
    void commands.workflowExport(id).then(
      (r) => {
        if (!alive) return;
        if (r.status === "ok") setText(r.data);
        else setError(r.error);
      },
      (e) => alive && setError(errorOf(e)),
    );
    return () => {
      alive = false;
    };
  }, [id]);

  const copy = async () => {
    if (text === null) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied("ok");
    } catch {
      setCopied("failed");
    }
  };

  const toFile = async () => {
    if (!item) return;
    setError(null);
    try {
      const path = await save({
        defaultPath: `${item.name.replace(/[\\/:*?"<>|]+/g, "_").trim() || "ablauf"}.json`,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const r = await commands.workflowExportFile(item.id, path);
      if (r.status === "ok") setSavedTo(path);
      else setError(r.error);
    } catch (e) {
      setError(errorOf(e) || t("automations.errors.generic"));
    }
  };

  return (
    <Dialog
      open={item !== null}
      onOpenChange={(o) => !o && onClose()}
      title={t("automations.export.title", { name: item?.name ?? "" })}
      description={t("automations.export.description")}
      closeLabel={t("automations.close")}
      className="max-w-2xl"
      footer={
        <Button size="sm" variant="secondary" onClick={onClose}>
          {t("automations.close")}
        </Button>
      }
    >
      <div className="space-y-3" data-testid="export-dialog">
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            disabled={text === null}
            onClick={() => void copy()}
            data-testid="export-copy"
          >
            {copied === "ok" ? (
              <Check size={14} aria-hidden="true" />
            ) : (
              <Copy size={14} aria-hidden="true" />
            )}
            {copied === "ok"
              ? t("automations.export.copied")
              : t("automations.export.copy")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={text === null}
            onClick={() => void toFile()}
            data-testid="export-file"
          >
            <Save size={14} aria-hidden="true" />
            {t("automations.export.toFile")}
          </Button>
        </div>
        {copied === "failed" && (
          <p className="text-xs text-status-amber" role="status">
            {t("automations.export.copyFailed")}
          </p>
        )}
        {savedTo && (
          <p
            className="break-all text-xs"
            role="status"
            data-testid="export-saved"
          >
            {t("automations.export.saved", { path: savedTo })}
          </p>
        )}
        {error && (
          <p className="text-sm text-status-red" role="alert">
            {error}
          </p>
        )}
        <label htmlFor="export-text" className="sr-only">
          {t("automations.export.text")}
        </label>
        <Textarea
          id="export-text"
          className="w-full font-mono"
          rows={14}
          readOnly
          spellCheck={false}
          value={text ?? ""}
          data-testid="export-text"
        />
      </div>
    </Dialog>
  );
};
