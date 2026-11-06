import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ArrowRight,
  Check,
  ChevronsRight,
  ChevronsLeft,
  Merge,
  MoreHorizontal,
  Scissors,
  MoveVertical,
} from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { ActionMenu, type ActionMenuItem } from "../ui/ActionMenu";
import {
  alignmentFor,
  audioChapters,
  deviceId,
  editionById,
  groupsFit,
  langName,
  newAlignment,
  textChapters,
  type ChapterList,
} from "@/lib/library/editions";
import {
  canConfirmAll,
  proposeGroups,
  validGroups,
  type AlignGroup,
} from "@/lib/library/languageSwitch";
import {
  canMoveBoundary,
  canSplit,
  confirmAll,
  confirmedCount,
  confirmGroup,
  joinWithNext,
  moveBoundary,
  splitGroup,
  type Side,
} from "@/lib/library/alignEdit";
import { saveAlignment } from "@/lib/library/editionOps";
import { loadPageText } from "@/lib/library/switching";
import "./sprachfassungen.css";

type Medium = "text" | "audio";

interface Props {
  book: LibraryBook | null;
  /** Das Paar, dessen Kapitel zugeordnet werden (Fassungskennungen). */
  pair: { a: string; b: string } | null;
  onClose(): void;
  onSaved(book: LibraryBook): void | Promise<void>;
}

/** Kapitel einer Fassung im gewählten Medium. */
async function chapters(
  book: LibraryBook,
  id: string,
  medium: Medium,
): Promise<ChapterList | null> {
  const e = editionById(book, id);
  if (!e) return null;
  if (medium === "audio") return e.audio ? audioChapters(e) : null;
  if (!e.text) return null;
  const text = await loadPageText(e.text.page_id);
  return text === null ? null : textChapters(text);
}

const labels = (list: ChapterList | null, idx: number[]) =>
  idx.map((i) => `${i + 1} ${list?.titles[i] ?? ""}`.trim());

/**
 * Dialog „Kapitel zuordnen“ (sf-08): Gruppen 1:1, 1:n und n:1 zwischen zwei
 * Fassungen. Jede Gruppe gilt erst nach „Bestätigen“; „Alle bestätigen“ gibt
 * es nur bei gleicher Kapitelzahl und lauter 1:1-Gruppen (Spezifikation §6.1),
 * sonst muss jede Gruppe einzeln bestätigt werden.
 */
export const AlignDialog: React.FC<Props> = ({
  book,
  pair,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const open = !!book && !!pair;
  const a = book && pair ? editionById(book, pair.a) : null;
  const b = book && pair ? editionById(book, pair.b) : null;
  const both = (m: Medium) =>
    m === "text" ? !!a?.text && !!b?.text : !!a?.audio && !!b?.audio;
  const [medium, setMedium] = useState<Medium>("text");
  const [lists, setLists] = useState<{
    a: ChapterList | null;
    b: ChapterList | null;
  } | null>(null);
  const [groups, setGroups] = useState<AlignGroup[]>([]);
  const [busy, setBusy] = useState(false);
  // Ein Neuladen der Bibliothek (Sync) darf angefangene Änderungen nicht verwerfen.
  const bookRef = useRef(book);
  bookRef.current = book;

  // Beim Öffnen: Text bevorzugen, sonst Hörbuch.
  useEffect(() => {
    if (!open) return;
    setMedium(both("text") ? "text" : "audio");
  }, [open, pair?.a, pair?.b]);

  useEffect(() => {
    const current = bookRef.current;
    if (!open || !current || !pair) return;
    let stale = false;
    setLists(null);
    void Promise.all([
      chapters(current, pair.a, medium),
      chapters(current, pair.b, medium),
    ]).then(([ca, cb]) => {
      if (stale) return;
      setLists({ a: ca, b: cb });
      const na = ca?.titles.length ?? 0;
      const nb = cb?.titles.length ?? 0;
      const saved = alignmentFor(current, pair.a, pair.b).groups;
      setGroups(groupsFit(saved, na, nb) ? saved : proposeGroups(na, nb));
    });
    return () => {
      stale = true;
    };
  }, [open, book?.id, pair?.a, pair?.b, medium]);

  const na = lists?.a?.titles.length ?? 0;
  const nb = lists?.b?.titles.length ?? 0;
  const done = confirmedCount(groups);
  const allAllowed = canConfirmAll(groups, na, nb);
  const anyOpen = done < groups.length;
  const nameA = langName(a?.lang);
  const nameB = langName(b?.lang);

  const save = async () => {
    if (!book || !pair || !validGroups(groups)) return;
    setBusy(true);
    try {
      const saved = await saveAlignment(
        book.id,
        newAlignment(pair.a, pair.b, groups, deviceId()),
      );
      toast(t("library.sf.align.saved"));
      onClose();
      await onSaved(saved);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const menu = (i: number): ActionMenuItem[] => {
    const g = groups[i];
    const edit = (next: AlignGroup[]) => setGroups(next);
    const bounds = (side: Side, forward: boolean): ActionMenuItem => ({
      id: `${side}-${forward ? "next" : "prev"}`,
      label: t(`library.sf.align.move.${side}.${forward ? "next" : "prev"}`),
      icon: MoveVertical,
      disabled: !canMoveBoundary(groups, i, side, forward),
      testId: `sf-align-move-${side}-${forward ? "next" : "prev"}-${i}`,
      onSelect: () => edit(moveBoundary(groups, i, side, forward)),
    });
    return [
      {
        id: "split",
        label: t("library.sf.align.split"),
        icon: Scissors,
        disabled: !g || !canSplit(g),
        testId: `sf-align-split-${i}`,
        onSelect: () => edit(splitGroup(groups, i)),
      },
      {
        id: "join",
        label: t("library.sf.align.join"),
        icon: Merge,
        disabled: i >= groups.length - 1,
        testId: `sf-align-join-${i}`,
        onSelect: () => edit(joinWithNext(groups, i)),
      },
      { ...bounds("a", true), dividerBefore: true },
      bounds("a", false),
      bounds("b", true),
      bounds("b", false),
    ];
  };

  const rows = useMemo(
    () =>
      groups.map((g, i) => ({
        g,
        i,
        left: labels(lists?.a ?? null, g.a),
        right: labels(lists?.b ?? null, g.b),
      })),
    [groups, lists],
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onClose()}
      title={t("library.sf.align.title")}
      closeLabel={t("library.cancel")}
      className="sf-align-dialog max-w-3xl"
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("library.cancel")}
          </Button>
          <Button
            disabled={busy || !allAllowed || !anyOpen}
            aria-describedby="sf-align-why"
            onClick={() => setGroups(confirmAll(groups))}
            data-testid="sf-align-confirm-all"
          >
            {t("library.sf.align.confirmAll")}
          </Button>
          <Button
            variant="primary"
            disabled={busy || !groups.length}
            onClick={() => void save()}
            data-testid="sf-align-save"
          >
            {t("library.sf.align.save")}
          </Button>
        </>
      }
    >
      <div className="sf-align" data-testid="sf-align-dialog">
        <p>
          {t("library.sf.align.intro", {
            title: book?.title ?? "",
            langA: nameA,
            countA: na,
            langB: nameB,
            countB: nb,
          })}
        </p>
        {both("text") && both("audio") && (
          <div
            className="sf-align__medium"
            role="group"
            aria-label={t("library.sf.align.mediumLabel")}
          >
            {(["text", "audio"] as const).map((m) => (
              <button
                key={m}
                type="button"
                className="sf-chip"
                aria-pressed={medium === m}
                onClick={() => setMedium(m)}
                data-testid={`sf-align-medium-${m}`}
              >
                {t(`library.sf.align.medium.${m}`)}
              </button>
            ))}
          </div>
        )}
        {!lists ? (
          <p className="sf-meta" role="status">
            {t("library.sf.align.loading")}
          </p>
        ) : !na || !nb ? (
          <p className="sf-meta" data-testid="sf-align-none">
            {t("library.sf.align.noChapters")}
          </p>
        ) : (
          <>
            <p>
              <b data-testid="sf-align-count">
                {t("library.sf.align.count", {
                  done,
                  total: groups.length,
                  countA: na,
                  countB: nb,
                })}
              </b>
            </p>
            <div
              className="sf-groups"
              role="list"
              aria-label={t("library.sf.align.list")}
            >
              {rows.map(({ g, i, left, right }) => (
                <div
                  className="sf-group"
                  role="listitem"
                  data-open={!g.confirmed || undefined}
                  data-testid="sf-align-group"
                  key={`${g.a[0]}-${g.b[0]}-${i}`}
                >
                  <div className="sf-group__side">
                    {left.map((l) => (
                      <div key={l}>{l}</div>
                    ))}
                  </div>
                  <span className="sf-group__arrow" aria-hidden="true">
                    {g.a.length === g.b.length ? (
                      <ArrowRight size={14} />
                    ) : g.b.length > g.a.length ? (
                      <ChevronsRight size={14} />
                    ) : (
                      <ChevronsLeft size={14} />
                    )}
                  </span>
                  <div className="sf-group__side">
                    {right.map((r) => (
                      <div key={r}>{r}</div>
                    ))}
                  </div>
                  <div className="sf-group__state">
                    {g.confirmed ? (
                      <span className="sf-ok">
                        <Check size={14} aria-hidden="true" />{" "}
                        {t("library.sf.align.confirmed")}
                      </span>
                    ) : (
                      <Button
                        aria-label={t("library.sf.align.confirmGroup", {
                          n: i + 1,
                        })}
                        onClick={() => setGroups(confirmGroup(groups, i))}
                        data-testid={`sf-align-confirm-${i}`}
                      >
                        {t("library.sf.align.confirm")}
                      </Button>
                    )}
                  </div>
                  <span className="library-touch">
                    <ActionMenu
                      trigger={{
                        icon: MoreHorizontal,
                        label: t("library.sf.align.edit", { n: i + 1 }),
                        description: t("library.sf.align.editMenu", {
                          n: i + 1,
                        }),
                        testId: `sf-align-edit-${i}`,
                        ghost: true,
                      }}
                      menuLabel={t("library.sf.align.editMenu", { n: i + 1 })}
                      align="end"
                      widthClass="w-72"
                      items={menu(i)}
                    />
                  </span>
                </div>
              ))}
            </div>
          </>
        )}
        <p id="sf-align-why" className="sf-meta sf-align__why">
          {t("library.sf.align.why")}
        </p>
      </div>
    </Dialog>
  );
};
