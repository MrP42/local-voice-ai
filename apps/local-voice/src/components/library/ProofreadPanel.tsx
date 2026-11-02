import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { FileDiff, RotateCcw, SpellCheck, X } from "lucide-react";
import { commands, type LibraryBook } from "@/bindings";
import { openSettingsTab } from "@/lib/openSettingsTab";
import {
  acceptEdits,
  buildReview,
  hashText,
  leadingParagraphs,
  pageText,
  rejectEdits,
  type DiffOp,
  type ParagraphEdit,
  type ProposalItem,
  type Review,
} from "@/lib/library/proofread";
import {
  cancelProofread,
  editParagraphs,
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
const Side: React.FC<{ ops: DiffOp[]; side: "before" | "after" }> = ({
  ops,
  side,
}) => (
  <p className="library-diff__text" data-side={side}>
    {ops
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

/** Text eines Buchs in der Änderungsansicht: Material des Kerns + aktueller Text. */
async function loadReview(book: LibraryBook): Promise<{
  review: Review | null;
  lead: number;
} | null> {
  const pageId = book.text?.page_id;
  if (!pageId) return null;
  const [material, current] = await Promise.all([
    commands.libraryProofreadMaterial(book.id),
    commands.pageStateLoad(pageId),
  ]);
  if (
    material.status !== "ok" ||
    material.data.original === null ||
    current.status !== "ok"
  )
    return null;
  const page = pageText(current.data);
  const corrected =
    material.data.corrected ??
    (hashText(page) === book.text?.proofread?.text_hash ? page : null);
  if (corrected === null) return { review: null, lead: 0 };
  return {
    review: buildReview(
      material.data.original,
      corrected,
      material.data.proposals,
      page,
    ),
    lead: leadingParagraphs(material.data.original),
  };
}

/**
 * Änderungsansicht: je korrigiertem Absatz Vorher/Nachher mit Annehmen und
 * Ablehnen, dazu „Alle annehmen/ablehnen“ und die von der Schutzprüfung
 * verworfenen Vorschläge als eigene Gruppe. Von Hand bearbeitete Absätze sind
 * gesperrt.
 */
const DiffDialog: React.FC<{
  book: LibraryBook;
  open: boolean;
  onClose(): void;
  onChanged(): Promise<void>;
}> = ({ book, open, onClose, onChanged }) => {
  const { t } = useTranslation();
  const [state, setState] = useState<{
    review: Review | null;
    lead: number;
  } | null>(null);
  const [shown, setShown] = useState(PAGE);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [accepted, setAccepted] = useState<Set<number>>(new Set());
  const [adopting, setAdopting] = useState<ProposalItem | null>(null);
  const pageId = book.text?.page_id;

  const reload = React.useCallback(async () => {
    const loaded = await loadReview(book);
    if (!loaded) setFailed(true);
    else setState(loaded);
  }, [book]);

  useEffect(() => {
    if (!open || !pageId) return;
    let alive = true;
    setState(null);
    setFailed(false);
    setShown(PAGE);
    setAccepted(new Set());
    void loadReview(book).then((loaded) => {
      if (!alive) return;
      if (!loaded) setFailed(true);
      else setState(loaded);
    });
    return () => {
      alive = false;
    };
    // Nur beim Öffnen und Buchwechsel laden; Änderungen laden selbst nach.
  }, [open, book.id, pageId]);

  const apply = async (edits: ParagraphEdit[]) => {
    if (busy || edits.length === 0) return;
    setBusy(true);
    try {
      const done = await editParagraphs(book.id, edits);
      if (done.skipped.length > 0)
        toast(
          t("library.proofread.diffEdited", { count: done.skipped.length }),
        );
      await onChanged();
      await reload();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const review = state?.review ?? null;
  const lead = state?.lead ?? 0;
  const number = (index: number) => Math.max(1, index - lead + 1);
  const canReject =
    !!review && review.items.some((i) => i.state === "corrected");
  const canAccept =
    !!review && review.items.some((i) => i.state === "rejected");
  const markAccepted = (list: number[]) =>
    setAccepted((prev) => new Set([...prev, ...list]));

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(o) => !o && onClose()}
        title={t("library.proofread.diffTitle", { title: book.title })}
        closeLabel={t("library.proofread.close")}
        className="lv-proofread-dialog max-w-4xl"
      >
        <div className="library-diff" data-testid="proofread-diff">
          {failed && (
            <p className="library-meta">
              {t("library.proofread.error.noOriginal")}
            </p>
          )}
          {!failed && state === null && (
            <p className="library-meta">{t("library.proofread.diffLoading")}</p>
          )}
          {state && !review && (
            <p
              className="library-proofread__warning"
              role="alert"
              data-testid="proofread-structure"
            >
              {t("library.proofread.diffStructure")}
            </p>
          )}
          {review && (
            <>
              <p className="library-meta" data-testid="proofread-diff-note">
                {t("library.proofread.diffNote")}
              </p>
              {review.edited > 0 && (
                <p
                  className="library-proofread__warning"
                  role="alert"
                  data-testid="proofread-edited-warning"
                >
                  {t("library.proofread.diffEdited", { count: review.edited })}
                </p>
              )}
              {review.items.length === 0 && review.proposals.length === 0 && (
                <p className="library-meta">
                  {t("library.proofread.diffNone")}
                </p>
              )}
              {review.items.length > 0 && (
                <>
                  <div className="library-diff__bar">
                    <span className="library-meta">
                      {t("library.proofread.diffCount", {
                        count: review.items.length,
                      })}
                    </span>
                    <div className="library-diff__bulk">
                      <Button
                        disabled={busy || !canAccept}
                        onClick={() => {
                          markAccepted(review.items.map((i) => i.index));
                          void apply(acceptEdits(review));
                        }}
                        data-testid="proofread-accept-all"
                      >
                        {t("library.proofread.acceptAll")}
                      </Button>
                      <Button
                        disabled={busy || !canReject}
                        onClick={() => void apply(rejectEdits(review))}
                        data-testid="proofread-reject-all"
                      >
                        {t("library.proofread.rejectAll")}
                      </Button>
                    </div>
                  </div>
                  <ol className="library-diff__list">
                    {review.items.slice(0, shown).map((c) => (
                      <li
                        key={c.index}
                        className="library-diff__item"
                        data-testid="proofread-diff-item"
                        data-state={
                          c.state === "corrected" && accepted.has(c.index)
                            ? "accepted"
                            : c.state
                        }
                      >
                        <div className="library-diff__head">
                          <span className="library-diff__no">
                            {t("library.proofread.paragraph", {
                              n: number(c.index),
                            })}
                          </span>
                          <span
                            className="library-diff__badge"
                            data-testid="proofread-item-state"
                          >
                            {c.state === "edited"
                              ? t("library.proofread.stateEdited")
                              : c.state === "rejected"
                                ? t("library.proofread.stateRejected")
                                : accepted.has(c.index)
                                  ? t("library.proofread.stateAccepted")
                                  : t("library.proofread.stateCorrected")}
                          </span>
                        </div>
                        <div className="library-diff__pair">
                          <div>
                            <span className="library-diff__label">
                              {t("library.proofread.before")}
                            </span>
                            <Side ops={c.ops} side="before" />
                          </div>
                          <div>
                            <span className="library-diff__label">
                              {t("library.proofread.after")}
                            </span>
                            <Side ops={c.ops} side="after" />
                          </div>
                        </div>
                        {c.state === "edited" && (
                          <p
                            className="library-meta"
                            data-testid="proofread-item-locked"
                          >
                            {t("library.proofread.lockedHint")}
                          </p>
                        )}
                        {c.state === "corrected" && (
                          <div className="library-diff__actions">
                            {!accepted.has(c.index) && (
                              <Button
                                disabled={busy}
                                onClick={() => markAccepted([c.index])}
                                data-testid="proofread-accept"
                              >
                                {t("library.proofread.accept")}
                              </Button>
                            )}
                            <Button
                              disabled={busy}
                              onClick={() =>
                                void apply([
                                  {
                                    index: c.index,
                                    expect: c.corrected,
                                    next: c.original,
                                  },
                                ])
                              }
                              data-testid="proofread-reject"
                            >
                              {t("library.proofread.reject")}
                            </Button>
                          </div>
                        )}
                        {c.state === "rejected" && (
                          <div className="library-diff__actions">
                            <Button
                              disabled={busy}
                              onClick={() =>
                                void apply([
                                  {
                                    index: c.index,
                                    expect: c.original,
                                    next: c.corrected,
                                  },
                                ])
                              }
                              data-testid="proofread-reaccept"
                            >
                              {t("library.proofread.reaccept")}
                            </Button>
                          </div>
                        )}
                      </li>
                    ))}
                  </ol>
                  {shown < review.items.length && (
                    <Button onClick={() => setShown((n) => n + PAGE)}>
                      {t("library.proofread.more", {
                        count: review.items.length - shown,
                      })}
                    </Button>
                  )}
                </>
              )}
              {review.proposals.length > 0 && (
                <section
                  className="library-diff__group"
                  data-testid="proofread-proposals"
                >
                  <h3 className="library-diff__group-title">
                    {t("library.proofread.proposalsTitle")} (
                    {review.proposals.length})
                  </h3>
                  <p className="library-meta">
                    {t("library.proofread.proposalsNote")}
                  </p>
                  <ol className="library-diff__list">
                    {review.proposals.map((c) => (
                      <li
                        key={c.index}
                        className="library-diff__item"
                        data-testid="proofread-proposal-item"
                        data-state={c.state}
                      >
                        <div className="library-diff__head">
                          <span className="library-diff__no">
                            {t("library.proofread.paragraph", {
                              n: number(c.index),
                            })}
                          </span>
                          <span className="library-diff__badge">
                            {c.state === "edited"
                              ? t("library.proofread.stateEdited")
                              : c.state === "adopted"
                                ? t("library.proofread.stateProposalAdopted")
                                : t("library.proofread.stateProposalOpen")}
                          </span>
                        </div>
                        <div className="library-diff__pair">
                          <div>
                            <span className="library-diff__label">
                              {t("library.proofread.original")}
                            </span>
                            <Side ops={c.ops} side="before" />
                          </div>
                          <div>
                            <span className="library-diff__label">
                              {t("library.proofread.proposal")}
                            </span>
                            <Side ops={c.ops} side="after" />
                          </div>
                        </div>
                        {c.state === "edited" && (
                          <p className="library-meta">
                            {t("library.proofread.lockedHint")}
                          </p>
                        )}
                        {c.state === "open" && (
                          <div className="library-diff__actions">
                            <Button
                              disabled={busy}
                              onClick={() => setAdopting(c)}
                              data-testid="proofread-adopt"
                            >
                              {t("library.proofread.adopt")}
                            </Button>
                          </div>
                        )}
                        {c.state === "adopted" && (
                          <div className="library-diff__actions">
                            <Button
                              disabled={busy}
                              onClick={() =>
                                void apply([
                                  {
                                    index: c.index,
                                    expect: c.proposal,
                                    next: c.original,
                                  },
                                ])
                              }
                              data-testid="proofread-adopt-revert"
                            >
                              {t("library.proofread.adoptRevert")}
                            </Button>
                          </div>
                        )}
                      </li>
                    ))}
                  </ol>
                </section>
              )}
            </>
          )}
        </div>
      </Dialog>
      <Dialog
        open={adopting !== null}
        onOpenChange={(o) => !o && setAdopting(null)}
        title={t("library.proofread.adoptTitle")}
        closeLabel={t("library.cancel")}
        className="lv-proofread-dialog"
        footer={
          <>
            <Button onClick={() => setAdopting(null)}>
              {t("library.cancel")}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                const c = adopting;
                setAdopting(null);
                if (c)
                  void apply([
                    { index: c.index, expect: c.original, next: c.proposal },
                  ]);
              }}
              data-testid="proofread-adopt-confirm"
            >
              {t("library.proofread.adoptConfirm")}
            </Button>
          </>
        }
      >
        <p>{t("library.proofread.adoptBody")}</p>
      </Dialog>
    </>
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

  const skippedHere = !!proof.skipped[book.id];
  // Anbieter immer frisch: ein Wechsel in den Einstellungen gilt sofort, auch
  // für den Hinweis, wohin der Text geht.
  useEffect(() => {
    let alive = true;
    const refresh = () =>
      void providerInfo().then((p) => alive && setProvider(p));
    refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      alive = false;
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [book.id, skippedHere]);

  const info = book.text?.proofread ?? null;
  const running = proof.current?.bookId === book.id ? proof.current : null;
  const waiting = proof.waiting.includes(book.id);
  const error = proof.errors[book.id];
  const working = running !== null || waiting;

  // Wurde der Text nach dem Korrekturlesen bearbeitet (Prüfsumme weicht ab)?
  const [edited, setEdited] = useState(false);
  const pageId = book.text?.page_id;
  const hash = info?.text_hash ?? null;
  useEffect(() => {
    if (!pageId || !hash || working) {
      setEdited(false);
      return;
    }
    let alive = true;
    void commands.pageStateLoad(pageId).then((r) => {
      if (alive)
        setEdited(r.status === "ok" && hashText(pageText(r.data)) !== hash);
    });
    return () => {
      alive = false;
    };
  }, [pageId, hash, working]);

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
      const done = await restoreOriginal(book.id);
      await onChanged();
      toast(
        done.kept > 0
          ? t("library.proofread.restoredPartial", { count: done.kept })
          : t("library.proofread.restored"),
      );
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
          <span style={{ display: "block" }}>
            {t("library.proofread.changed", { count: info.changed })}
          </span>
          {(info.unchanged ?? 0) > 0 && (
            <span
              className="library-meta"
              style={{ display: "block" }}
              data-testid="proofread-unchanged"
            >
              {t("library.proofread.unchanged", { count: info.unchanged })}
            </span>
          )}
          {info.rejected > 0 && (
            <span
              className="library-proofread__warning"
              style={{ display: "block" }}
              data-testid="proofread-rejected"
            >
              {t("library.proofread.rejected", { count: info.rejected })}
            </span>
          )}
          <span className="library-meta" style={{ display: "block" }}>
            {t("library.proofread.by", { model: info.model, when })}
          </span>
        </div>
      )}
      {!compact && !working && info && edited && (
        <span
          className="library-proofread__warning"
          role="alert"
          data-testid="proofread-edited-notice"
        >
          {t("library.proofread.editedNotice")}
        </span>
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
      {skippedHere && !working && (
        <span className="library-meta" data-testid="proofread-auto-skipped">
          {t("library.proofread.autoSkipped")}
        </span>
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
      <DiffDialog
        book={book}
        open={diff}
        onClose={() => setDiff(false)}
        onChanged={onChanged}
      />
      <Dialog
        open={confirm}
        onOpenChange={setConfirm}
        title={t("library.proofread.restoreTitle")}
        closeLabel={t("library.cancel")}
        className="lv-proofread-dialog"
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
