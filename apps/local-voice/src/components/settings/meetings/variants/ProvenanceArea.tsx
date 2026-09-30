import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ProvenanceEntry, type SubjectKind } from "@/bindings";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { ContextMenu } from "../search/FolderChips";

/** Wovon die Herkunft gezeigt wird. */
export type ProvenanceSubject =
  | {
      type: "transcript";
      meetingId: string;
      /** Kennung der aktiven Fassung, falls bekannt. */
      variantId: string | null;
    }
  | {
      type: "document";
      meetingId: string;
      /** `minutes` (Protokoll mit Zusammenfassung) oder `enhanced_notes` (KI-Notizen). */
      docKind: "minutes" | "enhanced_notes";
    };

/** Liest die Herkunft des Inhalts; leer, wenn es keine gibt. */
async function loadEntries(
  subject: ProvenanceSubject,
): Promise<ProvenanceEntry[]> {
  const get = async (kind: SubjectKind, id: string) => {
    const result = await commands.provenanceGet(kind, id);
    if (result.status !== "ok") throw new Error(result.error);
    return result.data ?? [];
  };
  if (subject.type === "transcript") {
    if (subject.variantId) {
      const own = await get("transcript_variant", subject.variantId);
      if (own.length > 0) return own;
    }
    return get("transcript", subject.meetingId);
  }
  const docs = await commands.meetingsGetDocuments(subject.meetingId);
  if (docs.status !== "ok") throw new Error(docs.error);
  const latest = (docs.data ?? [])
    .filter((d) => d.kind === subject.docKind)
    .sort((a, b) => b.version - a.version)[0];
  return latest ? get("document", latest.id) : [];
}

const formatDuration = (ms: number) =>
  ms < 60_000
    ? `${(ms / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s`
    : `${Math.floor(ms / 60_000)}:${Math.floor((ms % 60_000) / 1000)
        .toString()
        .padStart(2, "0")} min`;

const Row: React.FC<{
  label: string;
  testId: string;
  children: React.ReactNode;
}> = ({ label, testId, children }) => (
  <div
    className="grid grid-cols-[7rem_1fr] gap-2 py-1 text-sm"
    data-testid={testId}
  >
    <dt className="text-text/60">{label}</dt>
    <dd className="min-w-0 break-words">{children}</dd>
  </div>
);

interface ProvenanceDialogProps {
  subject: ProvenanceSubject;
  onClose: () => void;
}

/** Dialog „Herkunft“: Modell, Token, Dauer, Zeitpunkt, Quellen, Konfidenz, Ausloeser. */
export const ProvenanceDialog: React.FC<ProvenanceDialogProps> = ({
  subject,
  onClose,
}) => {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<ProvenanceEntry[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setEntries(null);
    setFailed(false);
    loadEntries(subject).then(
      (list) => !cancelled && setEntries(list),
      () => !cancelled && setFailed(true),
    );
    return () => {
      cancelled = true;
    };
  }, [subject]);

  const dash = t("meetings.provenance.unknown");
  return (
    <Dialog
      open
      title={t("meetings.provenance.title")}
      onOpenChange={(open) => !open && onClose()}
      closeLabel={t("meetings.provenance.close")}
      footer={
        <Button size="sm" variant="secondary" onClick={onClose}>
          {t("meetings.provenance.close")}
        </Button>
      }
    >
      <div data-testid="provenance-dialog" className="space-y-3">
        {failed ? (
          <p className="text-sm text-red-400">
            {t("meetings.provenance.failed")}
          </p>
        ) : entries === null ? (
          <p className="text-sm text-text/60">
            {t("meetings.provenance.loading")}
          </p>
        ) : entries.length === 0 ? (
          <p className="text-sm text-text/70" data-testid="provenance-empty">
            {t("meetings.provenance.empty")}
          </p>
        ) : (
          entries.map((e) => (
            <dl
              key={e.id}
              data-testid="provenance-entry"
              className="divide-y divide-mid-gray/15 rounded-md border border-mid-gray/20 px-3 py-1"
            >
              <Row
                label={t("meetings.provenance.operation")}
                testId="prov-operation"
              >
                {e.operation}
              </Row>
              <Row label={t("meetings.provenance.model")} testId="prov-model">
                {e.model_label ?? e.model_id ?? dash}
                {e.provider ? ` · ${e.provider}` : ""}
                {e.locality
                  ? ` · ${t(e.locality === "local" ? "meetings.provenance.local" : "meetings.provenance.remote")}`
                  : ""}
              </Row>
              <Row label={t("meetings.provenance.tokens")} testId="prov-tokens">
                {e.prompt_tokens !== null || e.completion_tokens !== null
                  ? t("meetings.provenance.tokensValue", {
                      input: e.prompt_tokens ?? 0,
                      output: e.completion_tokens ?? 0,
                    })
                  : dash}
              </Row>
              <Row
                label={t("meetings.provenance.duration")}
                testId="prov-duration"
              >
                {e.duration_ms !== null ? formatDuration(e.duration_ms) : dash}
              </Row>
              <Row label={t("meetings.provenance.time")} testId="prov-time">
                {new Date(e.created_at).toLocaleString(i18n.language)}
              </Row>
              <Row
                label={t("meetings.provenance.sources")}
                testId="prov-sources"
              >
                {e.sources.length === 0 ? (
                  dash
                ) : (
                  <ul className="space-y-0.5">
                    {e.sources.map((s, i) => (
                      <li key={`${s.kind}-${s.ref}-${i}`}>
                        {s.title ?? s.ref}{" "}
                        <span className="text-text/50">({s.kind})</span>
                      </li>
                    ))}
                  </ul>
                )}
              </Row>
              {e.confidence !== null && (
                <Row
                  label={t("meetings.provenance.confidence")}
                  testId="prov-confidence"
                >
                  {Math.round(e.confidence * 100)} %
                </Row>
              )}
              <Row
                label={t("meetings.provenance.trigger")}
                testId="prov-trigger"
              >
                {e.actor_kind
                  ? t(`meetings.provenance.actor.${e.actor_kind}`)
                  : dash}
              </Row>
              <Row label={t("meetings.provenance.origin")} testId="prov-origin">
                {e.origin === "recorded"
                  ? t("meetings.provenance.originRecorded")
                  : t("meetings.provenance.originDerived")}
              </Row>
            </dl>
          ))
        )}
      </div>
    </Dialog>
  );
};

interface ProvenanceAreaProps {
  subject: ProvenanceSubject;
  children: React.ReactNode;
  className?: string;
  testId?: string;
}

/**
 * Macht einen Bereich zum Ziel des Rechtsklicks „Herkunft“. In Eingabefeldern
 * bleibt das Menue des Systems (Kopieren, Einfuegen) unberuehrt.
 */
export const ProvenanceArea: React.FC<ProvenanceAreaProps> = ({
  subject,
  children,
  className,
  testId,
}) => {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [open, setOpen] = useState(false);
  return (
    <div
      className={className}
      data-testid={testId}
      onContextMenu={(e) => {
        const target = e.target as HTMLElement;
        if (target.closest("textarea, input, select, [contenteditable='true']"))
          return;
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      {children}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={t("meetings.provenance.menuLabel")}
          items={[
            {
              label: t("meetings.provenance.menu"),
              onSelect: () => setOpen(true),
            },
          ]}
          onClose={() => setMenu(null)}
        />
      )}
      {open && (
        <ProvenanceDialog subject={subject} onClose={() => setOpen(false)} />
      )}
    </div>
  );
};
