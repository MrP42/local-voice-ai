import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type LibraryBook } from "@/bindings";
import { LanguageSwitch } from "./LanguageSwitch";
import { SwitchNoticeHost } from "./SwitchNoticeHost";
import { clock } from "./usePlayerLanguage";
import {
  editionById,
  editionsOf,
  langName,
  langShort,
} from "@/lib/library/editions";
import {
  applySwitch,
  loadPageText,
  planSwitch,
  undoSwitch,
} from "@/lib/library/switching";
import { saveResumePoint } from "@/lib/reader/resumePoint";
import { showNotice } from "@/lib/library/notice";

interface Props {
  /** Seite, die der Reader gerade zeigt. */
  page: string;
  /** Der gezeigte Text (Original). */
  text: string;
  /** Aktuelle Stelle im Text (Zeichen). */
  getOffset(): number;
  /** Eine andere Seite im Reader öffnen. */
  onOpen(page: string): void;
}

const hasPage = (e: ReturnType<typeof editionsOf>[number], page: string) =>
  e.text?.page_id === page;

/**
 * Sprachknopf des Text-Lesers (sf-05): wechselt auf die Textfassung der anderen
 * Sprache an der umgerechneten Stelle und meldet „kapitelgenau“ bzw.
 * „ungefähr“ mit „Rückgängig“ (sf-06, sf-07). Erscheint nur bei Büchern mit
 * mehreren Sprachfassungen.
 */
export const ReaderLanguage: React.FC<Props> = ({
  page,
  text,
  getOffset,
  onOpen,
}) => {
  const { t } = useTranslation();
  const [book, setBook] = useState<LibraryBook | null>(null);
  const [focus, setFocus] = useState(false);

  const load = useCallback(async () => {
    const r = await commands.libraryList().catch(() => null);
    if (!r || r.status !== "ok") return null;
    const found =
      (r.data ?? []).find((b) => editionsOf(b).some((e) => hasPage(e, page))) ??
      null;
    setBook(found);
    return found;
  }, [page]);

  useEffect(() => {
    void load();
  }, [load]);

  const current = book
    ? (editionsOf(book).find((e) => hasPage(e, page)) ?? null)
    : null;
  if (!book || !current || editionsOf(book).length < 2) return null;

  const pick = async (toId: string) => {
    const target = editionById(book, toId);
    if (!target?.text) return;
    const offset = getOffset();
    try {
      const plan = await planSwitch(
        book,
        current.id,
        target.id,
        "text",
        offset,
        text.length,
      );
      if (!plan) throw new Error(t("library.sf.notice.failed"));
      const targetText = await loadPageText(target.text.page_id);
      const { book: updated, undo } = await applySwitch(
        book,
        plan,
        { offset, unit: "chars" },
        { wasPlaying: false, text: targetText },
      );
      const fromPage = current.text?.page_id ?? page;
      setFocus(true);
      setBook(updated);
      onOpen(target.text.page_id);
      const lang = langName(target.lang) || langShort(target.lang);
      const chapter = plan.level === "chapter";
      showNotice({
        owner: "reader",
        tone: chapter ? "info" : "warn",
        text: chapter
          ? t("library.sf.notice.chapter", {
              lang,
              chapter:
                plan.chapters.length > 1
                  ? `${plan.chapters[0] + 1}–${plan.chapters[plan.chapters.length - 1] + 1}`
                  : String((plan.chapters[0] ?? 0) + 1),
              total: plan.chapterTotal,
              at: plan.within
                ? t("library.sf.notice.at", {
                    at: clock(plan.within.at),
                    length: clock(plan.within.length),
                  })
                : "",
            })
          : t("library.sf.notice.approx", {
              lang,
              percent: Math.round(plan.fraction * 100),
            }),
        actions: [
          {
            id: "undo",
            label: t("library.sf.notice.undo"),
            run: async () => {
              try {
                await undoSwitch(undo);
                if (offset > 0)
                  saveResumePoint(`${fromPage}:original`, text, offset);
                setFocus(true);
                onOpen(fromPage);
                await load();
              } catch (e) {
                toast.error(e instanceof Error ? e.message : String(e));
              }
            },
          },
        ],
      });
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <span className="sf-reader" data-testid="sf-reader-language">
      <LanguageSwitch
        book={book}
        currentId={current.id}
        medium="text"
        placement="down"
        focusOnMount={focus}
        onPick={(id) => void pick(id)}
      />
      <SwitchNoticeHost owner="reader" />
    </span>
  );
};
