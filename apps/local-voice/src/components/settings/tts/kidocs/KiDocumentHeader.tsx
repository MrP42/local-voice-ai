import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ArrowLeft,
  Download,
  FileDown,
  FileText,
  FileType,
} from "lucide-react";
import type { KiDocument } from "@/bindings";
import { ActionMenu } from "../../../ui/ActionMenu";
import { Button } from "../../../ui/Button";
import type { KiRun } from "./useKiDocuments";
import { exportKiDocument, kiErrorText, type KiExportFormat } from "./kiDocs";
import "./kidocs.css";

/**
 * Kopfzeile eines offenen KI-Dokuments (lva#183, Must-fix 1 und 2): der Weg
 * zurück zur Liste, Titel, „Wird vorgelesen“ und der Export. Ein Auftrag, der
 * im Hintergrund läuft, bleibt als kurze Zeile sichtbar.
 */
export function KiDocumentHeader({
  pageId,
  doc,
  playing,
  run,
  onBack,
  onBeforeExport,
}: {
  pageId: string;
  doc: KiDocument;
  playing: boolean;
  run: KiRun | null;
  onBack: () => void;
  /** Ausstehende Änderungen sichern; exportiert wird der gesicherte Stand. */
  onBeforeExport: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const doExport = async (format: KiExportFormat) => {
    try {
      await onBeforeExport();
      if (await exportKiDocument(pageId, doc, format, t))
        toast.success(t("tts.kiDocs.export.done"));
    } catch (e) {
      toast.error(kiErrorText(String((e as Error).message ?? e), t));
    }
  };
  return (
    <div className="flex flex-col gap-1" data-testid="kidoc-head">
      <div className="kidoc-head">
        <Button
          variant="ghost"
          size="sm"
          icon={<ArrowLeft className="h-4 w-4" aria-hidden="true" />}
          onClick={onBack}
          data-testid="kidoc-back"
        >
          {t("tts.kiDocs.back")}
        </Button>
        <span className="kidoc-head__title" title={doc.title}>
          {doc.title}
        </span>
        {playing && (
          <span className="kidocs__badge text-xs" data-testid="kidoc-playing">
            {t("tts.kiDocs.playing")}
          </span>
        )}
        <span className="kidocs__meta">
          {doc.template_title} · {doc.source_label}
        </span>
        <ActionMenu
          trigger={{
            icon: Download,
            label: t("tts.kiDocs.export.title"),
            testId: "kidoc-export",
            description: t("tts.kiDocs.export.hint"),
          }}
          menuLabel={t("tts.kiDocs.export.title")}
          align="end"
          items={[
            {
              id: "md",
              label: t("tts.kiDocs.export.md"),
              icon: FileText,
              onSelect: () => void doExport("md"),
            },
            {
              id: "docx",
              label: t("tts.kiDocs.export.docx"),
              icon: FileType,
              onSelect: () => void doExport("docx"),
            },
            {
              id: "pdf",
              label: t("tts.kiDocs.export.pdf"),
              icon: FileDown,
              onSelect: () => void doExport("pdf"),
            },
          ]}
        />
      </div>
      {run && (
        <p className="kidocs__status" aria-live="polite">
          {t("tts.kiDocs.runningElsewhere", {
            template: run.templateTitle,
            done: Math.min(run.done, run.total),
            total: run.total,
          })}
        </p>
      )}
    </div>
  );
}
