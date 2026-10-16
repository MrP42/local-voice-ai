import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type DocTypeInfo, type IntegrationView } from "@/bindings";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Alert } from "../../../ui/Alert";
import { Dropdown } from "../../../ui/Dropdown";

/** Zusätzliche Formate je Dokument (Markdown ist immer dabei). */
export const EXTRA_FORMATS = ["docx", "pdf", "pptx"] as const;
const FORMAT_LABEL: Record<(typeof EXTRA_FORMATS)[number], string> = {
  docx: "Word",
  pdf: "PDF",
  pptx: "PowerPoint",
};

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  /** Eingerichtete und eingeschaltete Presenter-Integrationen. */
  targets: IntegrationView[];
  /** Dokumenttypen, zu denen es in dieser Besprechung ein Dokument gibt. */
  available: DocTypeInfo[];
}

/**
 * "An WAI Presenter senden" (Etappe 4a): Ziel, Dokumente und Formate wählen; das Backend
 * schreibt das Paket (`wai.besprechung/1`) samt Katalog in den Austauschordner.
 */
export const PresenterSendDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  meetingId,
  targets,
  available,
}) => {
  const { t } = useTranslation();
  const [target, setTarget] = useState<string>(
    targets[0]?.integration.id ?? "",
  );
  const [docs, setDocs] = useState<string[]>([]);
  const [formats, setFormats] = useState<string[]>(["pdf"]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setDocs(available.map((d) => d.key));
    setTarget((current) => current || targets[0]?.integration.id || "");
    setResult(null);
    setError(null);
  }, [open, available, targets]);

  const toggle = (list: string[], key: string) =>
    list.includes(key) ? list.filter((k) => k !== key) : [...list, key];

  const send = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    const res = await commands.presenterSend(target, meetingId, docs, formats);
    setBusy(false);
    if (res.status === "ok") {
      setResult(
        t("meetings.presenter.done", {
          folder: res.data.folder,
          files: res.data.files,
        }),
      );
    } else {
      setError(t("meetings.presenter.error", { error: res.error }));
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.presenter.title")}
      description={t("meetings.presenter.description")}
      closeLabel={t("integrations.close")}
      footer={
        <Button
          onClick={() => void send()}
          disabled={busy || !target}
          data-testid="presenter-send-submit"
        >
          {busy
            ? t("meetings.presenter.sending")
            : t("meetings.presenter.submit")}
        </Button>
      }
    >
      <div className="space-y-4 text-sm" data-testid="presenter-send-dialog">
        {targets.length > 1 && (
          <div className="space-y-1">
            <p className="font-medium">{t("meetings.presenter.target")}</p>
            <Dropdown
              options={targets.map((v) => ({
                value: v.integration.id,
                label: v.integration.label,
              }))}
              selectedValue={target}
              onSelect={setTarget}
            />
          </div>
        )}
        <div className="space-y-1">
          <p className="font-medium">{t("meetings.presenter.documents")}</p>
          {available.length === 0 && (
            <p className="text-text/60">
              {t("meetings.presenter.noDocuments")}
            </p>
          )}
          {available.map((d) => (
            <label key={d.key} className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={docs.includes(d.key)}
                onChange={() => setDocs((l) => toggle(l, d.key))}
                data-testid={`presenter-doc-${d.key}`}
              />
              <span>{d.title}</span>
            </label>
          ))}
        </div>
        <div className="space-y-1">
          <p className="font-medium">{t("meetings.presenter.formats")}</p>
          <div className="flex flex-wrap gap-4">
            {EXTRA_FORMATS.map((f) => (
              <label key={f} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={formats.includes(f)}
                  onChange={() => setFormats((l) => toggle(l, f))}
                  data-testid={`presenter-format-${f}`}
                />
                <span>{FORMAT_LABEL[f]}</span>
              </label>
            ))}
          </div>
        </div>
        {result && (
          <p
            className="break-all text-text/70"
            data-testid="presenter-send-done"
          >
            {result}
          </p>
        )}
        {error && <Alert variant="error">{error}</Alert>}
      </div>
    </Dialog>
  );
};
