import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FileDiff, RotateCcw, SpellCheck, X } from "lucide-react";
import { commands, type LibraryBook } from "@/bindings";
import { openSettingsTab } from "@/lib/openSettingsTab";
import {
  leadingParagraphs,
  paragraphChanges,
  pageText,
  type ParagraphChange,
} from "@/lib/library/proofread";
import {
  cancelProofread,
  isCancelled,
  proofreadBook,
  providerInfo,
  type ProviderInfo,
  restoreOriginal,
  setProofreadAuto,
  useProofread,
} from "@/lib/library/proofreadRun";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ProgressBar } from "./LibraryPage";

/** Wie viele Änderungen die Ansicht auf einmal zeigt. */
const PAGE = 60;

/** Alt: gleich + entfernt; Neu: gleich + hinzugefügt. Gestrichene und neue Wörter markiert. */
const Side: React.FC<{ change: ParagraphChange; side: "before" | "after" }> = ({
  change,
  side,
}) => (
  <p className="library-diff__text" data-side={side}>
    {change.ops
      .filter(
        (op) =>
          op.kind === "same" || op.kind === (side === "before" ? "del" : "add"),
      )
      .map((op, i) =>
        op.kind === "same" ? (
          <React.Fragment key={i}>{op.text}</React.Fragment>
        ) : op.kind === "del" ? (
          <del key={i}>{op.text}</del>
        ) : (
          <ins key={i}>{op.text}</ins>
        ),
      )}
  </p>
);

/** Absatz-Vergleich alt/neu mit markierten Wörtern. */
const DiffDialog: React.FC<{
  book: LibraryBook;
  open: boolean;
  onClose(): void;
}> = ({ book, open, onClose }) => {
  const { t } = useTranslation();
  const [changes, setChanges] = useState<ParagraphChange[] | null>(null);
  const [shown, setShown] = useState(PAGE);
  const [failed, setFailed] = useState(false);
  const [lead, setLead] = useState(0);
  const pageId = book.text?.page_id;
  useEffect(() => {
    if (!open || !pageId) return;
    let alive = true;
    setChanges(null);
    setFailed(false);
    setShown(PAGE);
    void (async () => {
      const [original, current] = await Promise.all([
        commands.libraryProofreadOriginal(book.id),
        commands.pageStateLoad(pageId),
      ]);
      if (!alive) return;
      if (
        original.status !== "ok" ||
        original.data === null ||
        current.status !== "ok"
      ) {
        setFailed(true);
        return;
      }
      setLead(leadingParagraphs(original.data));
      setChanges(paragraphChanges(original.data, pageText(current.data)));
    })();
    return () => {
      alive = false;
    };
  }, [open, book.id, pageId]);

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("library.proofread.diffTitle", { title: book.title })}
      closeLabel={t("library.proofread.close")}
      className="max-w-4xl"
    >
      <div className="library-diff" data-testid="proofread-diff">
        {failed && (
          <p className="library-meta">
            {t("library.proofread.error.noOriginal")}
          </p>
        )}
        {!failed && changes === null && (
          <p className="library-meta">{t("library.proofread.diffLoading")}</p>
        )}
        {changes?.length === 0 && (
          <p className="library-meta">{t("library.proofread.diffNone")}</p>
        )}
        {changes && changes.length > 0 && (
          <>
            <p className="library-meta">
              {t("library.proofread.diffCount", { count: changes.length })}
            </p>
            <ol className="library-diff__list">
              {changes.slice(0, shown).map((c) => (
                <li
                  key={c.index}
                  className="library-diff__item"
                  data-testid="proofread-diff-item"
                >
                  <span className="library-diff__no">
                    {t("library.proofread.paragraph", {
                      n: Math.max(1, c.index - lead + 1),
                    })}
                  </span>
                  <div className="library-diff__pair">
                    <div>
                      <span className="library-diff__label">
                        {t("library.proofread.before")}
                      </span>
                      <Side change={c} side="before" />
                    </div>
                    <div>
                      <span className="library-diff__label">
                        {t("library.proofread.after")}
                      </span>
                      <Side change={c} side="after" />
                    </div>
                  </div>
                </li>
              ))}
            </ol>
            {shown < changes.length && (
              <Button onClick={() => setShown((n) => n + PAGE)}>
                {t("library.proofread.more", { count: changes.length - shown })}
              </Button>
            )}
          </>
        )}
      </div>
    </Dialog>
  );
};

/**
 * Korrekturlesen an der Textfassung eines Transkripts (lva#208 B): Knopf,
 * Stand, Ergebnis, Änderungen ansehen, Original wiederherstellen.
 */
export const ProofreadPanel: React.FC<{
  book: LibraryBook;
  onChanged(): Promise<void>;
  /** Kurzfassung für die Übersicht: Stand und Knopf, ohne Erklärung und Werkzeuge. */
  compact?: boolean;
}> = ({ book, onChanged, compact = false }) => {
  const { t, i18n } = useTranslation();
  const proof = useProofread();
  const [provider, setProvider] = useState<ProviderInfo | null>(null);
  const ready = provider === null ? null : provider.ready;
  const [diff, setDiff] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    void providerInfo().then((p) => alive && setProvider(p));
    return () => {
      alive = false;
    };
  }, [book.id]);

  const info = book.text?.proofread ?? null;
  const running = proof.current?.bookId === book.id ? proof.current : null;
  const waiting = proof.waiting.includes(book.id);
  const error = proof.errors[book.id];
  const working = running !== null || waiting;

  // Fertig: die Bibliothek neu lesen, damit der Vermerk erscheint.
  const wasWorking = React.useRef(false);
  useEffect(() => {
    if (wasWorking.current && !working) void onChanged();
    wasWorking.current = working;
  }, [working, onChanged]);

  const start = async () => {
    try {
      await proofreadBook(book.id);
    } catch (e) {
      if (!isCancelled(e))
        toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const restore = async () => {
    setConfirm(false);
    setBusy(true);
    try {
      await restoreOriginal(book.id);
      await onChanged();
      toast(t("library.proofread.restored"));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const when = useMemo(
    () =>
      info
        ? new Date(info.at_ms).toLocaleString(i18n.language, {
            dateStyle: "medium",
            timeStyle: "short",
          })
        : "",
    [info, i18n.language],
  );

  if (!book.text || book.text.origin !== "transcript") return null;

  return (
    <div className="library-proofread" data-testid="proofread-panel">
      <div className="library-proofread__head">
        <SpellCheck size={14} aria-hidden="true" />
        <strong>{t("library.proofread.title")}</strong>
      </div>
      {running && (
        <div role="status" data-testid="proofread-status">
          <span className="library-meta">
            {running.total > 0
              ? t("library.proofread.running", {
                  done: running.done,
                  total: running.total,
                })
              : t("library.proofread.starting")}
          </span>
          <ProgressBar
            fraction={running.total > 0 ? running.done / running.total : 0}
            label={t("library.proofread.title")}
          />
        </div>
      )}
      {waiting && !running && (
        <span
          className="library-meta"
          role="status"
          data-testid="proofread-status"
        >
          {t("library.proofread.waiting")}
        </span>
      )}
      {!working && info && (
        <div role="status" data-testid="proofread-result">
          <span>
            {t("library.proofread.changed", { count: info.changed })}
            {info.rejected > 0 &&
              `, ${t("library.proofread.rejected", { count: info.rejected })}`}
          </span>
          <span className="library-meta" style={{ display: "block" }}>
            {t("library.proofread.by", { model: info.model, when })}
          </span>
        </div>
      )}
      {!compact && !working && !info && !error && (
        <span className="library-meta">{t("library.proofread.hint")}</span>
      )}
      {error && !working && (
        <span
          className="library-meta library-proofread__error"
          role="alert"
          data-testid="proofread-error"
        >
          {error}
        </span>
      )}
      {ready === false && !working && (
        <div
          className="library-proofread__notice"
          data-testid="proofread-no-provider"
        >
          <span className="library-meta">
            {t("library.proofread.noProvider")}
          </span>
          <Button size="sm" onClick={() => openSettingsTab("models")}>
            {t("library.proofread.openSettings")}
          </Button>
        </div>
      )}
      {provider?.ready && !provider.local && !working && (
        <span
          className="library-meta library-proofread__cloud"
          data-testid="proofread-cloud-notice"
        >
          {t("library.proofread.cloudNotice", { provider: provider.label })}
        </span>
      )}
      <div className="library-bar">
        {working ? (
          <Button
            icon={<X size={16} aria-hidden="true" />}
            onClick={() => void cancelProofread()}
            data-testid="proofread-cancel"
          >
            {t("library.proofread.cancel")}
          </Button>
        ) : (
          <Button
            icon={<SpellCheck size={16} aria-hidden="true" />}
            onClick={() => void start()}
            disabled={ready !== true || busy}
            data-testid="proofread-start"
          >
            {info ? t("library.proofread.again") : t("library.proofread.start")}
          </Button>
        )}
        {!compact && info && !working && (
          <>
            <Button
              variant="ghost"
              icon={<FileDiff size={16} aria-hidden="true" />}
              onClick={() => setDiff(true)}
              data-testid="proofread-show-diff"
            >
              {t("library.proofread.showChanges")}
            </Button>
            <Button
              variant="ghost"
              icon={<RotateCcw size={16} aria-hidden="true" />}
              onClick={() => setConfirm(true)}
              disabled={busy}
              data-testid="proofread-restore"
            >
              {t("library.proofread.restore")}
            </Button>
          </>
        )}
      </div>
      {!compact && (
        <label className="library-proofread__auto">
          <input
            type="checkbox"
            checked={proof.auto}
            onChange={(e) => setProofreadAuto(e.target.checked)}
            data-testid="proofread-auto"
          />
          {t("library.proofread.auto")}
        </label>
      )}
      <DiffDialog book={book} open={diff} onClose={() => setDiff(false)} />
      <Dialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t("library.proofread.restoreTitle")}
        closeLabel={t("library.cancel")}
        footer={
          <>
            <Button onClick={() => setConfirm(false)}>
              {t("library.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => void restore()}
              data-testid="proofread-restore-confirm"
            >
              {t("library.proofread.restore")}
            </Button>
          </>
        }
      >
        <p>{t("library.proofread.restoreBody")}</p>
      </Dialog>
    </div>
  );
};
