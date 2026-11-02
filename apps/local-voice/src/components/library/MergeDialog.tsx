import React from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { ArrowDown, ArrowUp } from "lucide-react";
import { IconButton } from "../ui/IconButton";
import { BookCover } from "./BookCover";
import type { SplitGroup } from "@/lib/library/audioGroups";
import { partCount, type MergePlan } from "@/lib/library/merge";

/**
 * Rückfrage vor dem Zusammenführen: Anzahl, Titel und die Reihenfolge der
 * Teile. Hat das Zielwerk schon ein Hörbuch, sagt der Dialog, dass es ersetzt
 * wird.
 */
export const MergeDialog: React.FC<{
  group: SplitGroup | null;
  plan: MergePlan | null;
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
  /** Reihenfolge ändern (Teil `from` an Stelle `to`). */
  onMove?(from: number, to: number): void;
}> = ({ group, plan, busy, onConfirm, onCancel, onMove }) => {
  const { t } = useTranslation();
  const open = !!group && !!plan;
  const total = group?.books.length ?? 0;
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onCancel()}
      title={t("library.merge.title", { title: plan?.title ?? "" })}
      closeLabel={t("library.cancel")}
      contentClassName="library-detail"
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            {t("library.cancel")}
          </Button>
          <Button
            variant="primary"
            onClick={onConfirm}
            disabled={busy}
            data-testid="library-merge-confirm"
          >
            {busy ? t("library.merge.working") : t("library.merge.confirm")}
          </Button>
        </>
      }
    >
      {group && plan && (
        <div className="library-merge" data-testid="library-merge-dialog">
          <div className="library-add__merge">
            <BookCover book={plan.target} size="xs" />
            <div style={{ minWidth: 0 }}>
              <p data-testid="library-merge-summary">
                {t("library.merge.summary", {
                  count: total,
                  title: plan.title,
                })}
              </p>
              {plan.existing && (
                <p className="library-meta" data-testid="library-merge-target">
                  {t("library.merge.intoExisting", {
                    title: plan.target.title,
                  })}
                </p>
              )}
              {plan.replacesAudio && (
                <p data-testid="library-merge-replaces">
                  <strong>
                    {t("library.merge.replaces", {
                      title: plan.target.title,
                      count: partCount(plan.target),
                    })}
                  </strong>
                </p>
              )}
            </div>
          </div>
          <p className="library-meta">{t("library.merge.orderHint")}</p>
          <ol
            className="library-merge__order"
            data-testid="library-merge-order"
          >
            {group.books.map((b, i) => (
              <li key={b.id} data-testid="library-merge-item">
                <span className="library-parts__no">{i + 1}</span>
                <span className="library-parts__title">
                  {b.title}
                  {b.text?.origin === "transcript" && (
                    <small className="library-meta">
                      {" · "}
                      {t("library.merge.hasTranscript")}
                    </small>
                  )}
                </span>
                {onMove && (
                  <span className="library-touch library-merge__move">
                    <IconButton
                      icon={<ArrowUp size={14} aria-hidden="true" />}
                      label={t("library.merge.up", { title: b.title })}
                      disabled={busy || i === 0}
                      onClick={() => onMove(i, i - 1)}
                    />
                    <IconButton
                      icon={<ArrowDown size={14} aria-hidden="true" />}
                      label={t("library.merge.down", { title: b.title })}
                      disabled={busy || i === group.books.length - 1}
                      onClick={() => onMove(i, i + 1)}
                    />
                  </span>
                )}
              </li>
            ))}
          </ol>
          {plan.transcripts !== "none" && (
            <p data-testid="library-merge-transcripts">
              {plan.transcripts === "join"
                ? t("library.merge.joinTranscripts")
                : t("library.merge.dropTranscripts", {
                    title: plan.target.title,
                  })}
            </p>
          )}
          <p className="library-meta">{t("library.merge.keepHint")}</p>
        </div>
      )}
    </Dialog>
  );
};
